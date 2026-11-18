//! `ISpatialAudioClient` on its own event-driven thread: the context's destination in the static
//! bed, each panner the page put in the `"spatial"` model on a dynamic object (or panned into the
//! bed, see `layout::objects_in_bed`).

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use redjs::webapi::audio::{OutputOpen, OutputRequest, OutputStream, Renderer};
use windows::core::{HSTRING, PCWSTR};
use windows::Media::Audio::{SpatialAudioDeviceConfiguration, SpatialAudioFormatSubtype};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioCategory_SoundEffects, AudioObjectType, AudioObjectType_Dynamic,
    IMMDeviceEnumerator, ISpatialAudioClient, ISpatialAudioObject, ISpatialAudioObjectRenderStream,
    MMDeviceEnumerator, SpatialAudioObjectRenderStreamActivationParams,
    SPTLAUDCLNT_E_NO_MORE_OBJECTS, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, BLOB, CLSCTX_ALL,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;

use crate::layout::{self, BedChannel};

/// Objects past this are not asked for: each holds a block of samples for every pass.
const MAX_OBJECTS: u32 = 64;
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

pub fn open(request: &OutputRequest, renderer: Renderer) -> Result<OutputOpen, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    let stop = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));
    let device = request.device_id.clone();
    let thread = {
        let (stop, error) = (Arc::clone(&stop), Arc::clone(&error));
        std::thread::Builder::new()
            .name("sk-spatial-audio".into())
            .spawn(move || {
                // SAFETY: COM for this thread only; every COM object below is dropped inside `run`.
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                let mut announced = false;
                if let Err(e) = run(device.as_deref(), renderer, &tx, &mut announced, &stop) {
                    if announced {
                        *error.lock().unwrap() = Some(e);
                    } else {
                        let _ = tx.send(Err(e));
                    }
                }
                if com {
                    unsafe { CoUninitialize() };
                }
            })
            .map_err(|e| format!("could not start the spatial audio thread: {e}"))?
    };
    let stream = SpatialStream {
        stop,
        error,
        thread: Some(thread),
    };
    match rx.recv_timeout(OPEN_TIMEOUT) {
        Ok(Ok(opened)) => Ok(OutputOpen {
            stream: Box::new(stream),
            sample_rate: opened.sample_rate,
            channels: opened.bed.len() as u32,
            channel_mask: layout::speaker_mask(&opened.bed),
            max_block_frames: opened.max_frames,
            max_objects: opened.max_objects,
            label: opened.label,
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the spatial audio renderer did not answer".into()),
    }
}

struct Opened {
    sample_rate: u32,
    bed: Vec<BedChannel>,
    max_frames: u32,
    max_objects: u32,
    label: String,
}

struct SpatialStream {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl OutputStream for SpatialStream {
    fn poll(&mut self) -> Result<(), String> {
        match self.error.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Drop for SpatialStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: created by CreateEventW, closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Dynamic {
    owner: u64,
    object: ISpatialAudioObject,
}

fn run(
    device: Option<&str>,
    mut renderer: Renderer,
    opened: &mpsc::SyncSender<Result<Opened, String>>,
    announced: &mut bool,
    stop: &AtomicBool,
) -> Result<(), String> {
    let hr = |what: &'static str| move |e: windows::core::Error| format!("{what} failed: {e}");
    // SAFETY: COM calls on this MTA thread; every pointer passed lives across the call.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(hr("the device enumerator"))?;
        let endpoint = match device {
            Some(id) => {
                let id = HSTRING::from(id);
                devices.GetDevice(PCWSTR(id.as_ptr()))
            }
            None => devices.GetDefaultAudioEndpoint(eRender, eConsole),
        }
        .map_err(hr("opening the output device"))?;
        let endpoint_id = endpoint.GetId().ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            s
        });
        let client: ISpatialAudioClient = endpoint
            .Activate(CLSCTX_INPROC_SERVER, None)
            .map_err(|e| format!("the device has no spatial audio renderer ({e})"))?;
        let dynamic = client
            .GetMaxDynamicObjectCount()
            .map_err(hr("GetMaxDynamicObjectCount"))?;
        if dynamic == 0 {
            return Err(
                "no spatial sound format (Windows Sonic, Dolby Atmos, DTS) is on for this device"
                    .into(),
            );
        }
        let formats = client
            .GetSupportedAudioObjectFormatEnumerator()
            .map_err(hr("the object format"))?;
        let format_ptr = formats.GetFormat(0).map_err(hr("the object format"))?;
        let format: WAVEFORMATEX = std::ptr::read_unaligned(format_ptr);
        let (bits, channels, rate) = (
            format.wBitsPerSample,
            format.nChannels,
            format.nSamplesPerSec,
        );
        if bits != 32 || channels != 1 {
            return Err(format!(
                "the renderer's object format ({channels} ch, {bits} bit) is not float mono"
            ));
        }
        let native = client
            .GetNativeStaticObjectTypeMask()
            .map(|m| m.0 as u32)
            .unwrap_or(0);
        let bed = layout::bed(native);
        let non_native = bed.iter().filter(|c| native & c.object_type == 0).count() as u32;
        let max_objects = dynamic.saturating_sub(non_native).min(MAX_OBJECTS);
        if max_objects == 0 {
            return Err("the spatial renderer has no dynamic objects to spare".into());
        }
        let max_frames = client
            .GetMaxFrameCount(format_ptr)
            .map_err(hr("GetMaxFrameCount"))?;
        let label = endpoint_id
            .as_deref()
            .map_or_else(|| "spatial".into(), format_label);
        let in_bed = layout::objects_in_bed(&label);

        let event = Event(CreateEventW(None, false, false, None).map_err(hr("CreateEventW"))?);
        let mut params = SpatialAudioObjectRenderStreamActivationParams {
            ObjectFormat: format_ptr,
            StaticObjectTypeMask: AudioObjectType(layout::object_type_mask(&bed) as i32),
            MinDynamicObjectCount: 0,
            MaxDynamicObjectCount: if in_bed { 0 } else { max_objects },
            Category: AudioCategory_SoundEffects,
            EventHandle: event.0,
            NotifyObject: ManuallyDrop::new(None),
        };
        // Not dropped: PROPVARIANT's Drop is PropVariantClear, which would free `params` (a stack value).
        let blob = ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB {
                            cbSize: size_of::<SpatialAudioObjectRenderStreamActivationParams>()
                                as u32,
                            pBlobData: &mut params as *mut _ as *mut u8,
                        },
                    },
                }),
            },
        });
        let stream: ISpatialAudioObjectRenderStream = client
            .ActivateSpatialAudioStream(&*blob)
            .map_err(hr("activating the spatial audio stream"))?;
        let statics = bed
            .iter()
            .map(|c| stream.ActivateSpatialAudioObject(AudioObjectType(c.object_type as i32)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(hr("activating the static bed"))?;

        *announced = true;
        if opened
            .send(Ok(Opened {
                sample_rate: rate,
                bed: bed.clone(),
                max_frames,
                max_objects,
                label: label.clone(),
            }))
            .is_err()
        {
            return Ok(());
        }

        let mut task = 0u32;
        let mmcss = AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut task).ok();
        stream
            .Start()
            .map_err(hr("starting the spatial audio stream"))?;
        let result = render_loop(
            &stream,
            &statics,
            &mut renderer,
            &bed,
            in_bed,
            max_frames,
            max_objects,
            &event,
            stop,
            &label,
        );
        let _ = stream.Stop();
        if let Some(h) = mmcss {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
        result
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn render_loop(
    stream: &ISpatialAudioObjectRenderStream,
    statics: &[ISpatialAudioObject],
    renderer: &mut Renderer,
    bed: &[BedChannel],
    in_bed: bool,
    max_frames: u32,
    max_objects: u32,
    event: &Event,
    stop: &AtomicBool,
    label: &str,
) -> Result<(), String> {
    let channels = bed.len();
    let mut mix = vec![0.0f32; max_frames as usize * channels];
    let mut dynamic: Vec<Option<Dynamic>> = (0..max_objects).map(|_| None).collect();
    let mut seen = vec![false; max_objects as usize];
    let mut ran_out = false;
    let mut mono = vec![0.0f32; max_frames as usize];
    let mut panned: Vec<Option<(u64, Vec<f32>)>> = (0..max_objects).map(|_| None).collect();
    let mut target = vec![0.0f32; channels];
    while !stop.load(Ordering::Acquire) {
        if WaitForSingleObject(event.0, 200) != WAIT_OBJECT_0 {
            continue;
        }
        let (mut available, mut frames) = (0u32, 0u32);
        stream
            .BeginUpdatingAudioObjects(&mut available, &mut frames)
            .map_err(|e| format!("the spatial audio stream stopped: {e}"))?;
        let frames = (frames as usize).min(max_frames as usize);
        renderer.render(&mut mix, channels, frames);
        if in_bed {
            seen.fill(false);
            for block in renderer.objects() {
                let i = block.index as usize;
                let Some(slot) = panned.get_mut(i) else {
                    continue;
                };
                seen[i] = true;
                let gain = block.placement.gain;
                let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
                let out = &mut mono[..frames];
                layout::object_samples(block.left, block.right, block.channels, gain, out);
                layout::bed_gains(layout::listener_space(&block.placement), bed, &mut target);
                let fresh = !matches!(slot, Some((owner, _)) if *owner == block.owner);
                let prev = slot.get_or_insert_with(|| (block.owner, vec![0.0; channels]));
                if fresh {
                    prev.0 = block.owner;
                    prev.1.copy_from_slice(&target);
                }
                layout::mix_into_bed(
                    out,
                    &prev.1,
                    &target,
                    &mut mix[..frames * channels],
                    channels,
                );
                prev.1.copy_from_slice(&target);
            }
            for (slot, seen) in panned.iter_mut().zip(&seen) {
                if !seen {
                    *slot = None;
                }
            }
        }
        for (k, object) in statics.iter().enumerate() {
            if let Some(buf) = buffer(object, frames) {
                for (i, s) in buf.iter_mut().enumerate() {
                    *s = mix[i * channels + k];
                }
            }
        }

        seen.fill(false);
        for block in renderer.objects().filter(|_| !in_bed) {
            let i = block.index as usize;
            let Some(slot) = dynamic.get_mut(i) else {
                continue;
            };
            seen[i] = true;
            if slot.as_ref().is_some_and(|d| d.owner != block.owner) {
                if let Some(old) = slot.take() {
                    let _ = old.object.SetEndOfStream(0);
                }
            }
            if slot.is_none() {
                let activated = if available > 0 {
                    stream.ActivateSpatialAudioObject(AudioObjectType_Dynamic)
                } else {
                    Err(SPTLAUDCLNT_E_NO_MORE_OBJECTS.into())
                };
                match activated {
                    Ok(object) => {
                        available -= 1;
                        *slot = Some(Dynamic {
                            owner: block.owner,
                            object,
                        });
                    }
                    Err(e) => {
                        if !ran_out {
                            ran_out = true;
                            eprintln!(
                                "[audio] the {label} renderer has no object left for a PannerNode ({e}); it \
                                 renders with HRTF"
                            );
                        }
                        renderer.lose_object(block.index);
                        continue;
                    }
                }
            }
            let d = slot.as_ref().unwrap();
            let (volume, gain) = layout::object_gain(block.placement.gain);
            if let Some(buf) = buffer(&d.object, frames) {
                layout::object_samples(block.left, block.right, block.channels, gain, buf);
            }
            let [x, y, z] = layout::listener_space(&block.placement);
            let _ = d.object.SetPosition(x, y, z);
            let _ = d.object.SetVolume(volume);
        }
        for (slot, seen) in dynamic.iter_mut().zip(&seen) {
            if !seen {
                if let Some(old) = slot.take() {
                    let _ = old.object.SetEndOfStream(0);
                }
            }
        }
        stream
            .EndUpdatingAudioObjects()
            .map_err(|e| format!("the spatial audio stream stopped: {e}"))?;
    }
    Ok(())
}

/// The object's buffer for this pass, as `frames` samples.
unsafe fn buffer<'a>(object: &ISpatialAudioObject, frames: usize) -> Option<&'a mut [f32]> {
    let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
    object.GetBuffer(&mut ptr, &mut len).ok()?;
    if ptr.is_null() {
        return None;
    }
    let n = (len as usize / size_of::<f32>()).min(frames);
    Some(std::slice::from_raw_parts_mut(ptr.cast::<f32>(), n))
}

/// The spatial format on the endpoint, for `AudioContext.spatialAudio.format`.
fn format_label(endpoint: &str) -> String {
    let id = HSTRING::from(format!(
        r"\\?\SWD#MMDEVAPI#{endpoint}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}"
    ));
    let Some(active) = SpatialAudioDeviceConfiguration::GetForDeviceId(&id)
        .and_then(|c| c.ActiveSpatialAudioFormat())
        .ok()
    else {
        return "unknown".into();
    };
    type Subtype = fn() -> windows::core::Result<HSTRING>;
    let known: [(Subtype, &str); 7] = [
        (SpatialAudioFormatSubtype::WindowsSonic, "windows-sonic"),
        (
            SpatialAudioFormatSubtype::DolbyAtmosForHeadphones,
            "dolby-atmos-headphones",
        ),
        (
            SpatialAudioFormatSubtype::DolbyAtmosForHomeTheater,
            "dolby-atmos-home-theater",
        ),
        (
            SpatialAudioFormatSubtype::DolbyAtmosForSpeakers,
            "dolby-atmos-speakers",
        ),
        (SpatialAudioFormatSubtype::DTSHeadphoneX, "dts-headphone-x"),
        (SpatialAudioFormatSubtype::DTSXUltra, "dts-x-ultra"),
        (
            SpatialAudioFormatSubtype::DTSXForHomeTheater,
            "dts-x-home-theater",
        ),
    ];
    let active = active.to_string_lossy();
    known
        .iter()
        .find(|(f, _)| f().is_ok_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&active)))
        .map_or("unknown", |(_, name)| name)
        .into()
}
