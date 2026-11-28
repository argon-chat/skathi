//! Shared, read-only input from the four controller collections on Valve's wireless Puck.
use sk_windows::raw::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use sk_windows::raw::{
    CreateFileW, ReadFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use sk_windows::raw::{
    GetLastError, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE,
};
use sk_windows::raw::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID,
};
use sk_windows::raw::{
    HidD_FreePreparsedData, HidD_GetFeature, HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps,
    HIDP_CAPS, HIDP_STATUS_SUCCESS,
};
use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const PUCK: u16 = 0x1304;
const DISCOVER: Duration = Duration::from_secs(1);

fn controller_path(device: HANDLE) -> Option<Vec<u16>> {
    let mut size = size_of::<RID_DEVICE_INFO>() as u32;
    let mut info = RID_DEVICE_INFO {
        cbSize: size,
        ..Default::default()
    };
    unsafe {
        if GetRawInputDeviceInfoW(device, RIDI_DEVICEINFO, (&raw mut info).cast(), &mut size)
            == u32::MAX
            || info.dwType != RIM_TYPEHID
        {
            return None;
        }
        let hid = info.Anonymous.hid;
        if (
            hid.dwVendorId,
            hid.dwProductId,
            hid.usUsagePage,
            hid.usUsage,
        ) != (0x28de, PUCK.into(), 0xff00, 1)
        {
            return None;
        }
        size = 0;
        if GetRawInputDeviceInfoW(device, RIDI_DEVICENAME, null_mut(), &mut size) == u32::MAX
            || size == 0
        {
            return None;
        }
        let mut path = vec![0; size as usize + 1];
        if GetRawInputDeviceInfoW(device, RIDI_DEVICENAME, path.as_mut_ptr().cast(), &mut size)
            == u32::MAX
        {
            return None;
        }
        let hardware = String::from_utf16_lossy(&path).to_ascii_lowercase();
        let usb = hardware.split('#').nth(1)?;
        usb.split('&')
            .any(|part| matches!(part, "mi_02" | "mi_03" | "mi_04" | "mi_05"))
            .then_some(path)
    }
}

fn present_paths() -> Option<Vec<Vec<u16>>> {
    let mut length = 0;
    let size = size_of::<RAWINPUTDEVICELIST>() as u32;
    unsafe {
        if GetRawInputDeviceList(null_mut(), &mut length, size) == u32::MAX {
            return None;
        }
        let mut devices = vec![RAWINPUTDEVICELIST::default(); length as usize];
        let count = GetRawInputDeviceList(devices.as_mut_ptr(), &mut length, size);
        if count == u32::MAX {
            return None;
        }
        Some(
            devices
                .get(..count as usize)?
                .iter()
                .filter(|d| d.dwType == RIM_TYPEHID)
                .filter_map(|d| controller_path(d.hDevice))
                .collect(),
        )
    }
}

fn open_shared(path: &[u16], flags: u32) -> Option<OwnedHandle> {
    unsafe {
        let handle = CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        );
        (handle != INVALID_HANDLE_VALUE).then(|| OwnedHandle::from_raw_handle(handle))
    }
}

// IBEX GET_WIRELESS_STATE is a query on the Puck's dongle feature channel (report ID 2).
fn wireless_status(path: &[u16], length: usize) -> Option<Vec<u8>> {
    if length < 4 {
        return None;
    }
    let file = open_shared(path, 0)?;
    let mut bytes = vec![0u8; length];
    bytes[..2].copy_from_slice(&[2, 0xb4]);
    unsafe {
        if HidD_SetFeature(
            file.as_raw_handle(),
            bytes.as_mut_ptr().cast(),
            length as u32,
        ) == 0
        {
            return None;
        }
        bytes[1..].fill(0);
        if HidD_GetFeature(
            file.as_raw_handle(),
            bytes.as_mut_ptr().cast(),
            length as u32,
        ) == 0
        {
            return None;
        }
    }
    Some(bytes)
}

#[derive(Default)]
struct InitialConnection {
    observed: bool,
}

impl InitialConnection {
    fn observe(&mut self, input: &[u8]) {
        if !self.observed && super::report::decode(input).is_some() {
            self.observed = true;
        }
    }

    fn accepts(&self, reply: &[u8]) -> bool {
        !self.observed
            && reply.starts_with(&[2, 0xb4])
            && matches!(
                super::report::decode(reply),
                Some(super::report::Report::Connected)
            )
    }
}

struct Reader {
    id: u32,
    file: OwnedHandle,
    // Windows retains both pointers while a read is pending, even if the Reader moves.
    overlap: Box<UnsafeCell<OVERLAPPED>>,
    buffer: Vec<u8>,
    pending: bool,
    initial: InitialConnection,
}

impl Reader {
    fn open(path: &[u16], id: u32) -> Option<(Self, usize)> {
        let file = open_shared(path, FILE_FLAG_OVERLAPPED)?;
        let handle = file.as_raw_handle();
        unsafe {
            let mut data = 0;
            if HidD_GetPreparsedData(handle, &mut data) == 0 {
                return None;
            }
            let mut caps = HIDP_CAPS::default();
            let status = HidP_GetCaps(data, &mut caps);
            HidD_FreePreparsedData(data);
            if status != HIDP_STATUS_SUCCESS
                || (caps.UsagePage, caps.Usage) != (0xff00, 1)
                || caps.InputReportByteLength == 0
            {
                return None;
            }
            Some((
                Self {
                    id,
                    file,
                    overlap: Box::new(UnsafeCell::new(OVERLAPPED::default())),
                    buffer: vec![0; usize::from(caps.InputReportByteLength)],
                    pending: false,
                    initial: InitialConnection::default(),
                },
                usize::from(caps.FeatureReportByteLength),
            ))
        }
    }

    fn read(&mut self, mut report: impl FnMut(&[u8])) -> bool {
        let file = self.file.as_raw_handle();
        let overlap = self.overlap.get();
        for _ in 0..64 {
            let mut len = 0;
            unsafe {
                if !self.pending {
                    overlap.write(OVERLAPPED::default());
                    if ReadFile(
                        file,
                        self.buffer.as_mut_ptr(),
                        self.buffer.len() as u32,
                        null_mut(),
                        overlap,
                    ) == 0
                        && GetLastError() != ERROR_IO_PENDING
                    {
                        return false;
                    }
                    self.pending = true;
                }
                if GetOverlappedResult(file, overlap, &mut len, 0) == 0 {
                    if GetLastError() == ERROR_IO_INCOMPLETE {
                        return true;
                    }
                    self.pending = false;
                    return false;
                }
            }
            self.pending = false;
            if len == 0 {
                return false;
            }
            let bytes = &self.buffer[..len as usize];
            self.initial.observe(bytes);
            report(bytes);
        }
        true
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if self.pending {
            let mut len = 0;
            unsafe {
                // Cancellation only requests completion. The kernel must finish before the buffer is freed.
                CancelIoEx(self.file.as_raw_handle(), self.overlap.get());
                GetOverlappedResult(self.file.as_raw_handle(), self.overlap.get(), &mut len, 1);
            }
        }
    }
}

pub(super) struct Transport {
    readers: BTreeMap<Vec<u16>, Reader>,
    last_discovery: Option<Instant>,
    next_id: u32,
    status_tx: Sender<(u32, Vec<u8>)>,
    status_rx: Receiver<(u32, Vec<u8>)>,
}

impl Transport {
    pub(super) fn new() -> Self {
        let (status_tx, status_rx) = mpsc::channel();
        Self {
            readers: BTreeMap::new(),
            last_discovery: None,
            next_id: 0,
            status_tx,
            status_rx,
        }
    }

    /// Reports retain their HID report ID. A lost collection sends None once.
    pub(super) fn poll(&mut self, mut event: impl FnMut(u32, u16, &[u16], Option<&[u8]>)) {
        let now = Instant::now();
        if self
            .last_discovery
            .is_none_or(|last| now.duration_since(last) >= DISCOVER)
        {
            self.last_discovery = Some(now);
            if let Some(paths) = present_paths() {
                self.readers.retain(|path, reader| {
                    let present = paths.contains(path);
                    if !present {
                        event(reader.id, PUCK, path, None);
                    }
                    present
                });
                let mut queries = Vec::new();
                for path in paths {
                    if !self.readers.contains_key(&path)
                        && let Some(id) = self.next_id.checked_add(1)
                        && let Some((reader, feature_length)) = Reader::open(&path, id)
                    {
                        queries.push((id, path.clone(), feature_length));
                        self.readers.insert(path, reader);
                        self.next_id = id;
                    }
                }
                if !queries.is_empty() {
                    let replies = self.status_tx.clone();
                    let _ = std::thread::Builder::new()
                        .name("Puck connection status".into())
                        .spawn(move || {
                            for (id, path, length) in queries {
                                if let Some(reply) = wireless_status(&path, length)
                                    && replies.send((id, reply)).is_err()
                                {
                                    break;
                                }
                            }
                        });
                }
            }
        }
        self.readers.retain(|path, reader| {
            let id = reader.id;
            let live = reader.read(|bytes| event(id, PUCK, path, Some(bytes)));
            if !live {
                event(id, PUCK, path, None);
            }
            live
        });
        for (id, report) in self.status_rx.try_iter() {
            if let Some((path, _)) = self
                .readers
                .iter()
                .find(|(_, reader)| reader.id == id && reader.initial.accepts(&report))
            {
                event(id, PUCK, path, Some(&report));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_query_only_establishes_a_connection_before_live_input() {
        let mut connection = InitialConnection::default();
        connection.observe(&[0x42]);
        assert!(connection.accepts(&[2, 0xb4, 1, 2]));
        assert!(!connection.accepts(&[2, 0xb4, 1, 1]));
        assert!(!connection.accepts(&[2, 0xb4, 2, 2]));
        assert!(!connection.accepts(&[0x46, 2]));
        let mut battery = [0; 15];
        battery[..3].copy_from_slice(&[0x43, 1, 75]);
        connection.observe(&battery);
        assert!(!connection.accepts(&[2, 0xb4, 1, 2]));
        assert!(!connection.accepts(&[2, 0xb4, 1, 1]));
    }

    #[test]
    #[ignore = "requires a Valve controller Puck connected to Windows"]
    fn read_connected_puck() {
        let mut transport = Transport::new();
        let mut reports = BTreeMap::new();
        let mut pressed = std::collections::BTreeSet::new();
        let mut status = BTreeMap::new();
        let secs = std::env::var("SK_PUCK_PROBE_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let until = Instant::now() + Duration::from_secs(secs);
        transport.poll(|_, _, _, _| {});
        eprintln!(
            "Listening on {} Puck input collections for {secs}s",
            transport.readers.len()
        );
        while Instant::now() < until {
            transport.poll(|slot, _, _, bytes| {
                if let Some(bytes) = bytes {
                    *reports.entry((slot, bytes[0], bytes.len())).or_insert(0u32) += 1;
                    if bytes.starts_with(&[2, 0xb4]) {
                        status.insert(slot, bytes[3]);
                    }
                    if let Some(super::super::report::Report::State(state)) =
                        super::super::report::decode(bytes)
                    {
                        pressed.extend(
                            state.buttons[..17]
                                .iter()
                                .enumerate()
                                .filter_map(|(i, &v)| (v > 0.5).then_some(i)),
                        );
                    }
                }
            });
            std::thread::sleep(Duration::from_millis(5));
        }
        eprintln!("Puck input collections: {}; reports (slot, type, length): {reports:?}; wireless status: {status:?}; pressed standard buttons: {pressed:?}", transport.readers.len());
        assert!(
            !transport.readers.is_empty(),
            "No Puck controller collection opened"
        );
    }
}
