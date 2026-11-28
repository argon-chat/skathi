#[inline]
pub unsafe fn ActivateAudioInterfaceAsync<P0, P3>(
    deviceinterfacepath: P0,
    riid: *const windows_core::GUID,
    activationparams: Option<*const PROPVARIANT>,
    completionhandler: P3,
) -> windows_core::Result<IActivateAudioInterfaceAsyncOperation>
where
    P0: windows_core::Param<windows_core::PCWSTR>,
    P3: windows_core::Param<IActivateAudioInterfaceCompletionHandler>,
{
    windows_core::link!("mmdevapi.dll" "system" fn ActivateAudioInterfaceAsync(deviceinterfacepath : windows_core::PCWSTR, riid : *const windows_core::GUID, activationparams : *const PROPVARIANT, completionhandler : *mut core::ffi::c_void, activationoperation : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        ActivateAudioInterfaceAsync(
            deviceinterfacepath.param().abi(),
            riid,
            activationparams.unwrap_or(core::mem::zeroed()) as _,
            completionhandler.param().abi(),
            &mut result__,
        )
        .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn AvRevertMmThreadCharacteristics(avrthandle: HANDLE) -> windows_core::BOOL {
    windows_core::link!("avrt.dll" "system" fn AvRevertMmThreadCharacteristics(avrthandle : HANDLE) -> windows_core::BOOL);
    unsafe { AvRevertMmThreadCharacteristics(avrthandle) }
}
#[inline]
pub unsafe fn AvSetMmThreadCharacteristicsW<P0>(taskname: P0, taskindex: *mut u32) -> HANDLE
where
    P0: windows_core::Param<windows_core::PCWSTR>,
{
    windows_core::link!("avrt.dll" "system" fn AvSetMmThreadCharacteristicsW(taskname : windows_core::PCWSTR, taskindex : *mut u32) -> HANDLE);
    unsafe { AvSetMmThreadCharacteristicsW(taskname.param().abi(), taskindex as _) }
}
#[inline]
pub unsafe fn CloseHandle(hobject: HANDLE) -> windows_core::BOOL {
    windows_core::link!("kernel32.dll" "system" fn CloseHandle(hobject : HANDLE) -> windows_core::BOOL);
    unsafe { CloseHandle(hobject) }
}
#[inline]
pub unsafe fn CoCreateInstance<P1, T>(
    rclsid: *const windows_core::GUID,
    punkouter: P1,
    dwclscontext: CLSCTX,
) -> windows_core::Result<T>
where
    P1: windows_core::Param<windows_core::IUnknown>,
    T: windows_core::Interface,
{
    windows_core::link!("ole32.dll" "system" fn CoCreateInstance(rclsid : *const windows_core::GUID, punkouter : *mut core::ffi::c_void, dwclscontext : CLSCTX, riid : *const windows_core::GUID, ppv : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    let mut result__ = core::ptr::null_mut();
    unsafe {
        CoCreateInstance(
            rclsid,
            punkouter.param().abi(),
            dwclscontext,
            &T::IID,
            &mut result__,
        )
        .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn CoInitializeEx(
    pvreserved: Option<*const core::ffi::c_void>,
    dwcoinit: u32,
) -> windows_core::HRESULT {
    windows_core::link!("ole32.dll" "system" fn CoInitializeEx(pvreserved : *const core::ffi::c_void, dwcoinit : u32) -> windows_core::HRESULT);
    unsafe { CoInitializeEx(pvreserved.unwrap_or(core::mem::zeroed()) as _, dwcoinit) }
}
#[inline]
pub unsafe fn CoRegisterClassObject<P1>(
    rclsid: *const windows_core::GUID,
    punk: P1,
    dwclscontext: CLSCTX,
    flags: u32,
) -> windows_core::Result<u32>
where
    P1: windows_core::Param<windows_core::IUnknown>,
{
    windows_core::link!("ole32.dll" "system" fn CoRegisterClassObject(rclsid : *const windows_core::GUID, punk : *mut core::ffi::c_void, dwclscontext : CLSCTX, flags : u32, lpdwregister : *mut u32) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        CoRegisterClassObject(
            rclsid,
            punk.param().abi(),
            dwclscontext,
            flags,
            &mut result__,
        )
        .map(|| result__)
    }
}
#[inline]
pub unsafe fn CoRevokeClassObject(dwregister: u32) -> windows_core::HRESULT {
    windows_core::link!("ole32.dll" "system" fn CoRevokeClassObject(dwregister : u32) -> windows_core::HRESULT);
    unsafe { CoRevokeClassObject(dwregister) }
}
#[inline]
pub unsafe fn CoTaskMemFree(pv: Option<*const core::ffi::c_void>) {
    windows_core::link!("ole32.dll" "system" fn CoTaskMemFree(pv : *const core::ffi::c_void));
    unsafe { CoTaskMemFree(pv.unwrap_or(core::mem::zeroed()) as _) }
}
#[inline]
pub unsafe fn CoUninitialize() {
    windows_core::link!("ole32.dll" "system" fn CoUninitialize());
    unsafe { CoUninitialize() }
}
#[inline]
pub unsafe fn CreateEventW<P3>(
    lpeventattributes: Option<*const SECURITY_ATTRIBUTES>,
    bmanualreset: bool,
    binitialstate: bool,
    lpname: P3,
) -> HANDLE
where
    P3: windows_core::Param<windows_core::PCWSTR>,
{
    windows_core::link!("kernel32.dll" "system" fn CreateEventW(lpeventattributes : *const SECURITY_ATTRIBUTES, bmanualreset : windows_core::BOOL, binitialstate : windows_core::BOOL, lpname : windows_core::PCWSTR) -> HANDLE);
    unsafe {
        CreateEventW(
            lpeventattributes.unwrap_or(core::mem::zeroed()) as _,
            bmanualreset.into(),
            binitialstate.into(),
            lpname.param().abi(),
        )
    }
}
#[inline]
pub unsafe fn CreateStreamOnHGlobal(
    hglobal: HGLOBAL,
    fdeleteonrelease: bool,
) -> windows_core::Result<IStream> {
    windows_core::link!("ole32.dll" "system" fn CreateStreamOnHGlobal(hglobal : HGLOBAL, fdeleteonrelease : windows_core::BOOL, ppstm : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        CreateStreamOnHGlobal(hglobal, fdeleteonrelease.into(), &mut result__)
            .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn D3D11CreateDevice<P0>(
    padapter: P0,
    drivertype: D3D_DRIVER_TYPE,
    software: HMODULE,
    flags: D3D11_CREATE_DEVICE_FLAG,
    pfeaturelevels: Option<&[D3D_FEATURE_LEVEL]>,
    sdkversion: u32,
    ppdevice: Option<*mut Option<ID3D11Device>>,
    pfeaturelevel: Option<*mut D3D_FEATURE_LEVEL>,
    ppimmediatecontext: Option<*mut Option<ID3D11DeviceContext>>,
) -> windows_core::HRESULT
where
    P0: windows_core::Param<IDXGIAdapter>,
{
    windows_core::link!("d3d11.dll" "system" fn D3D11CreateDevice(padapter : *mut core::ffi::c_void, drivertype : D3D_DRIVER_TYPE, software : HMODULE, flags : D3D11_CREATE_DEVICE_FLAG, pfeaturelevels : *const D3D_FEATURE_LEVEL, featurelevels : u32, sdkversion : u32, ppdevice : *mut *mut core::ffi::c_void, pfeaturelevel : *mut D3D_FEATURE_LEVEL, ppimmediatecontext : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        D3D11CreateDevice(
            padapter.param().abi(),
            drivertype,
            software,
            flags,
            pfeaturelevels.map_or(core::ptr::null(), |slice| slice.as_ptr()),
            pfeaturelevels.map_or(0, |slice| slice.len().try_into().unwrap()),
            sdkversion,
            ppdevice.unwrap_or(core::mem::zeroed()) as _,
            pfeaturelevel.unwrap_or(core::mem::zeroed()) as _,
            ppimmediatecontext.unwrap_or(core::mem::zeroed()) as _,
        )
    }
}
#[inline]
pub unsafe fn DispatchMessageW(lpmsg: *const MSG) -> LRESULT {
    windows_core::link!("user32.dll" "system" fn DispatchMessageW(lpmsg : *const MSG) -> LRESULT);
    unsafe { DispatchMessageW(lpmsg) }
}
#[inline]
pub unsafe fn GlobalAlloc(uflags: GLOBAL_ALLOC_FLAGS, dwbytes: usize) -> HGLOBAL {
    windows_core::link!("kernel32.dll" "system" fn GlobalAlloc(uflags : GLOBAL_ALLOC_FLAGS, dwbytes : usize) -> HGLOBAL);
    unsafe { GlobalAlloc(uflags, dwbytes) }
}
#[inline]
pub unsafe fn GlobalFree(hmem: Option<HGLOBAL>) -> HGLOBAL {
    windows_core::link!("kernel32.dll" "system" fn GlobalFree(hmem : HGLOBAL) -> HGLOBAL);
    unsafe { GlobalFree(hmem.unwrap_or(core::mem::zeroed()) as _) }
}
#[inline]
pub unsafe fn GlobalLock(hmem: HGLOBAL) -> *mut core::ffi::c_void {
    windows_core::link!("kernel32.dll" "system" fn GlobalLock(hmem : HGLOBAL) -> *mut core::ffi::c_void);
    unsafe { GlobalLock(hmem) }
}
#[inline]
pub unsafe fn GlobalUnlock(hmem: HGLOBAL) -> windows_core::BOOL {
    windows_core::link!("kernel32.dll" "system" fn GlobalUnlock(hmem : HGLOBAL) -> windows_core::BOOL);
    unsafe { GlobalUnlock(hmem) }
}
#[inline]
pub unsafe fn ILCreateFromPathW<P0>(pszpath: P0) -> *mut ITEMIDLIST
where
    P0: windows_core::Param<windows_core::PCWSTR>,
{
    windows_core::link!("shell32.dll" "system" fn ILCreateFromPathW(pszpath : windows_core::PCWSTR) -> *mut ITEMIDLIST);
    unsafe { ILCreateFromPathW(pszpath.param().abi()) }
}
#[inline]
pub unsafe fn ILFree(pidl: Option<*const ITEMIDLIST>) {
    windows_core::link!("shell32.dll" "system" fn ILFree(pidl : *const ITEMIDLIST));
    unsafe { ILFree(pidl.unwrap_or(core::mem::zeroed()) as _) }
}
#[inline]
pub unsafe fn MFCreateAsyncResult<P0, P1, P2>(
    punkobject: P0,
    pcallback: P1,
    punkstate: P2,
) -> windows_core::Result<IMFAsyncResult>
where
    P0: windows_core::Param<windows_core::IUnknown>,
    P1: windows_core::Param<IMFAsyncCallback>,
    P2: windows_core::Param<windows_core::IUnknown>,
{
    windows_core::link!("mfplat.dll" "system" fn MFCreateAsyncResult(punkobject : *mut core::ffi::c_void, pcallback : *mut core::ffi::c_void, punkstate : *mut core::ffi::c_void, ppasyncresult : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        MFCreateAsyncResult(
            punkobject.param().abi(),
            pcallback.param().abi(),
            punkstate.param().abi(),
            &mut result__,
        )
        .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn MFCreateAttributes(
    ppmfattributes: *mut Option<IMFAttributes>,
    cinitialsize: u32,
) -> windows_core::HRESULT {
    windows_core::link!("mfplat.dll" "system" fn MFCreateAttributes(ppmfattributes : *mut *mut core::ffi::c_void, cinitialsize : u32) -> windows_core::HRESULT);
    unsafe { MFCreateAttributes(core::mem::transmute(ppmfattributes), cinitialsize) }
}
#[inline]
pub unsafe fn MFCreateAudioRenderer<P0>(paudioattributes: P0) -> windows_core::Result<IMFMediaSink>
where
    P0: windows_core::Param<IMFAttributes>,
{
    windows_core::link!("mf.dll" "system" fn MFCreateAudioRenderer(paudioattributes : *mut core::ffi::c_void, ppsink : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        MFCreateAudioRenderer(paudioattributes.param().abi(), &mut result__)
            .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn MFCreateDXGIDeviceManager(
    resettoken: *mut u32,
    ppdevicemanager: *mut Option<IMFDXGIDeviceManager>,
) -> windows_core::HRESULT {
    windows_core::link!("mfplat.dll" "system" fn MFCreateDXGIDeviceManager(resettoken : *mut u32, ppdevicemanager : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe { MFCreateDXGIDeviceManager(resettoken as _, core::mem::transmute(ppdevicemanager)) }
}
#[inline]
pub unsafe fn MFCreateMFByteStreamOnStream<P0>(pstream: P0) -> windows_core::Result<IMFByteStream>
where
    P0: windows_core::Param<IStream>,
{
    windows_core::link!("mfplat.dll" "system" fn MFCreateMFByteStreamOnStream(pstream : *mut core::ffi::c_void, ppbytestream : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe {
        let mut result__ = core::mem::zeroed();
        MFCreateMFByteStreamOnStream(pstream.param().abi(), &mut result__)
            .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn MFInvokeCallback<P0>(pasyncresult: P0) -> windows_core::HRESULT
where
    P0: windows_core::Param<IMFAsyncResult>,
{
    windows_core::link!("mfplat.dll" "system" fn MFInvokeCallback(pasyncresult : *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe { MFInvokeCallback(pasyncresult.param().abi()) }
}
#[inline]
pub unsafe fn MFPutWorkItem<P1, P2>(
    dwqueue: u32,
    pcallback: P1,
    pstate: P2,
) -> windows_core::HRESULT
where
    P1: windows_core::Param<IMFAsyncCallback>,
    P2: windows_core::Param<windows_core::IUnknown>,
{
    windows_core::link!("mfplat.dll" "system" fn MFPutWorkItem(dwqueue : u32, pcallback : *mut core::ffi::c_void, pstate : *mut core::ffi::c_void) -> windows_core::HRESULT);
    unsafe { MFPutWorkItem(dwqueue, pcallback.param().abi(), pstate.param().abi()) }
}
#[inline]
pub unsafe fn MFStartup(version: u32, dwflags: u32) -> windows_core::HRESULT {
    windows_core::link!("mfplat.dll" "system" fn MFStartup(version : u32, dwflags : u32) -> windows_core::HRESULT);
    unsafe { MFStartup(version, dwflags) }
}
#[inline]
pub unsafe fn MFTEnumEx(
    guidcategory: windows_core::GUID,
    flags: u32,
    pinputtype: Option<*const MFT_REGISTER_TYPE_INFO>,
    poutputtype: Option<*const MFT_REGISTER_TYPE_INFO>,
    pppmftactivate: *mut *mut Option<IMFActivate>,
    pnummftactivate: *mut u32,
) -> windows_core::HRESULT {
    windows_core::link!("mfplat.dll" "system" fn MFTEnumEx(guidcategory : windows_core::GUID, flags : u32, pinputtype : *const MFT_REGISTER_TYPE_INFO, poutputtype : *const MFT_REGISTER_TYPE_INFO, pppmftactivate : *mut *mut *mut core::ffi::c_void, pnummftactivate : *mut u32) -> windows_core::HRESULT);
    unsafe {
        MFTEnumEx(
            guidcategory,
            flags,
            pinputtype.unwrap_or(core::mem::zeroed()) as _,
            poutputtype.unwrap_or(core::mem::zeroed()) as _,
            pppmftactivate as _,
            pnummftactivate as _,
        )
    }
}
#[inline]
pub unsafe fn MapViewOfFile(
    hfilemappingobject: HANDLE,
    dwdesiredaccess: FILE_MAP,
    dwfileoffsethigh: u32,
    dwfileoffsetlow: u32,
    dwnumberofbytestomap: usize,
) -> MEMORY_MAPPED_VIEW_ADDRESS {
    windows_core::link!("kernel32.dll" "system" fn MapViewOfFile(hfilemappingobject : HANDLE, dwdesiredaccess : FILE_MAP, dwfileoffsethigh : u32, dwfileoffsetlow : u32, dwnumberofbytestomap : usize) -> MEMORY_MAPPED_VIEW_ADDRESS);
    unsafe {
        MapViewOfFile(
            hfilemappingobject,
            dwdesiredaccess,
            dwfileoffsethigh,
            dwfileoffsetlow,
            dwnumberofbytestomap,
        )
    }
}
#[inline]
pub unsafe fn PeekMessageW(
    lpmsg: *mut MSG,
    hwnd: Option<HWND>,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> windows_core::BOOL {
    windows_core::link!("user32.dll" "system" fn PeekMessageW(lpmsg : *mut MSG, hwnd : HWND, wmsgfiltermin : u32, wmsgfiltermax : u32, wremovemsg : PEEK_MESSAGE_REMOVE_TYPE) -> windows_core::BOOL);
    unsafe {
        PeekMessageW(
            lpmsg as _,
            hwnd.unwrap_or(core::mem::zeroed()) as _,
            wmsgfiltermin,
            wmsgfiltermax,
            wremovemsg,
        )
    }
}
#[inline]
pub unsafe fn RoInitialize(inittype: RO_INIT_TYPE) -> windows_core::HRESULT {
    windows_core::link!("api-ms-win-core-winrt-l1-1-0.dll" "system" fn RoInitialize(inittype : RO_INIT_TYPE) -> windows_core::HRESULT);
    unsafe { RoInitialize(inittype) }
}
#[inline]
pub unsafe fn SHCreateItemFromParsingName<P0, P1, T>(
    pszpath: P0,
    pbc: P1,
) -> windows_core::Result<T>
where
    P0: windows_core::Param<windows_core::PCWSTR>,
    P1: windows_core::Param<IBindCtx>,
    T: windows_core::Interface,
{
    windows_core::link!("shell32.dll" "system" fn SHCreateItemFromParsingName(pszpath : windows_core::PCWSTR, pbc : *mut core::ffi::c_void, riid : *const windows_core::GUID, ppv : *mut *mut core::ffi::c_void) -> windows_core::HRESULT);
    let mut result__ = core::ptr::null_mut();
    unsafe {
        SHCreateItemFromParsingName(
            pszpath.param().abi(),
            pbc.param().abi(),
            &T::IID,
            &mut result__,
        )
        .and_then(|| windows_core::imp::Type::from_abi(result__))
    }
}
#[inline]
pub unsafe fn SHOpenFolderAndSelectItems(
    pidlfolder: *const ITEMIDLIST,
    apidl: Option<&[*const ITEMIDLIST]>,
    dwflags: u32,
) -> windows_core::HRESULT {
    windows_core::link!("shell32.dll" "system" fn SHOpenFolderAndSelectItems(pidlfolder : *const ITEMIDLIST, cidl : u32, apidl : *const *const ITEMIDLIST, dwflags : u32) -> windows_core::HRESULT);
    unsafe {
        SHOpenFolderAndSelectItems(
            pidlfolder,
            apidl.map_or(0, |slice| slice.len().try_into().unwrap()),
            apidl.map_or(core::ptr::null(), |slice| slice.as_ptr()),
            dwflags,
        )
    }
}
#[inline]
pub unsafe fn TranslateMessage(lpmsg: *const MSG) -> windows_core::BOOL {
    windows_core::link!("user32.dll" "system" fn TranslateMessage(lpmsg : *const MSG) -> windows_core::BOOL);
    unsafe { TranslateMessage(lpmsg) }
}
#[inline]
pub unsafe fn WaitForSingleObject(hhandle: HANDLE, dwmilliseconds: u32) -> WAIT_EVENT {
    windows_core::link!("kernel32.dll" "system" fn WaitForSingleObject(hhandle : HANDLE, dwmilliseconds : u32) -> WAIT_EVENT);
    unsafe { WaitForSingleObject(hhandle, dwmilliseconds) }
}
pub type ADVANCED_FEATURE_FLAGS = u16;
pub const AUDCLNT_BUFFERFLAGS_SILENT: _AUDCLNT_BUFFERFLAGS = 2;
pub type AUDCLNT_SHAREMODE = i32;
pub const AUDCLNT_SHAREMODE_SHARED: AUDCLNT_SHAREMODE = 0;
pub const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 2147483648;
pub const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 262144;
pub const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 131072;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AUDIOCLIENT_ACTIVATION_PARAMS {
    pub ActivationType: AUDIOCLIENT_ACTIVATION_TYPE,
    pub Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0,
}
impl Default for AUDIOCLIENT_ACTIVATION_PARAMS {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union AUDIOCLIENT_ACTIVATION_PARAMS_0 {
    pub ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
}
impl Default for AUDIOCLIENT_ACTIVATION_PARAMS_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub type AUDIOCLIENT_ACTIVATION_TYPE = i32;
pub const AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK: AUDIOCLIENT_ACTIVATION_TYPE = 1;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
    pub TargetProcessId: u32,
    pub ProcessLoopbackMode: PROCESS_LOOPBACK_MODE,
}
pub type AUDIO_STREAM_CATEGORY = i32;
pub const AudioCategory_SoundEffects: AUDIO_STREAM_CATEGORY = 5;
pub type AudioObjectType = i32;
pub const AudioObjectType_Dynamic: AudioObjectType = 1;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BLOB {
    pub cbSize: u32,
    pub pBlobData: *mut u8,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BSTRBLOB {
    pub cbSize: u32,
    pub pData: *mut u8,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CABOOL {
    pub cElems: u32,
    pub pElems: *mut VARIANT_BOOL,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CABSTR {
    pub cElems: u32,
    pub pElems: *mut windows_core::BSTR,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CABSTRBLOB {
    pub cElems: u32,
    pub pElems: *mut BSTRBLOB,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAC {
    pub cElems: u32,
    pub pElems: windows_core::PSTR,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CACLIPDATA {
    pub cElems: u32,
    pub pElems: *mut CLIPDATA,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CACLSID {
    pub cElems: u32,
    pub pElems: *mut windows_core::GUID,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CACY {
    pub cElems: u32,
    pub pElems: *mut CY,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CADATE {
    pub cElems: u32,
    pub pElems: *mut f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CADBL {
    pub cElems: u32,
    pub pElems: *mut f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAFILETIME {
    pub cElems: u32,
    pub pElems: *mut FILETIME,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAFLT {
    pub cElems: u32,
    pub pElems: *mut f32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAH {
    pub cElems: u32,
    pub pElems: *mut i64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAI {
    pub cElems: u32,
    pub pElems: *mut i16,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAL {
    pub cElems: u32,
    pub pElems: *mut i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CALPSTR {
    pub cElems: u32,
    pub pElems: *mut windows_core::PSTR,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CALPWSTR {
    pub cElems: u32,
    pub pElems: *mut windows_core::PWSTR,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAPROPVARIANT {
    pub cElems: u32,
    pub pElems: *mut PROPVARIANT,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CASCODE {
    pub cElems: u32,
    pub pElems: *mut i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAUB {
    pub cElems: u32,
    pub pElems: *mut u8,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAUH {
    pub cElems: u32,
    pub pElems: *mut u64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAUI {
    pub cElems: u32,
    pub pElems: *mut u16,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CAUL {
    pub cElems: u32,
    pub pElems: *mut u32,
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CHAR(pub i8);
pub const CLASS_E_NOAGGREGATION: windows_core::HRESULT = windows_core::HRESULT(0x80040110_u32 as _);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CLIPDATA {
    pub cbSize: u32,
    pub ulClipFmt: i32,
    pub pClipData: *mut u8,
}
pub type CLSCTX = u32;
pub const CLSCTX_ALL: CLSCTX = 23;
pub const CLSCTX_INPROC_SERVER: CLSCTX = 1;
pub const CLSCTX_LOCAL_SERVER: CLSCTX = 4;
pub const CLSID_MFMediaEngineClassFactory: windows_core::GUID =
    windows_core::GUID::from_u128(0xb44392da_499b_446b_a4cb_005fead0e6d5);
pub type COINIT = i32;
pub const COINIT_APARTMENTTHREADED: COINIT = 2;
pub const COINIT_DISABLE_OLE1DDE: COINIT = 4;
pub const COINIT_MULTITHREADED: COINIT = 0;
#[repr(C)]
#[derive(Clone, Copy)]
pub union CY {
    pub Anonymous: CY_0,
    pub int64: i64,
}
impl Default for CY {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CY_0 {
    pub Lo: u32,
    pub Hi: i32,
}
pub type D3D11_BIND_FLAG = i32;
pub const D3D11_BIND_RENDER_TARGET: D3D11_BIND_FLAG = 32;
pub const D3D11_BIND_SHADER_RESOURCE: D3D11_BIND_FLAG = 8;
pub type D3D11_CPU_ACCESS_FLAG = i32;
pub const D3D11_CPU_ACCESS_READ: D3D11_CPU_ACCESS_FLAG = 131072;
pub const D3D11_CREATE_DEVICE_BGRA_SUPPORT: D3D11_CREATE_DEVICE_FLAG = 32;
pub type D3D11_CREATE_DEVICE_FLAG = u32;
pub const D3D11_CREATE_DEVICE_VIDEO_SUPPORT: D3D11_CREATE_DEVICE_FLAG = 2048;
pub type D3D11_MAP = i32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct D3D11_MAPPED_SUBRESOURCE {
    pub pData: *mut core::ffi::c_void,
    pub RowPitch: u32,
    pub DepthPitch: u32,
}
pub type D3D11_MAP_FLAG = i32;
pub const D3D11_MAP_FLAG_DO_NOT_WAIT: D3D11_MAP_FLAG = 1048576;
pub const D3D11_MAP_READ: D3D11_MAP = 1;
pub const D3D11_SDK_VERSION: u32 = 7;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct D3D11_SUBRESOURCE_DATA {
    pub pSysMem: *mut core::ffi::c_void,
    pub SysMemPitch: u32,
    pub SysMemSlicePitch: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct D3D11_TEXTURE2D_DESC {
    pub Width: u32,
    pub Height: u32,
    pub MipLevels: u32,
    pub ArraySize: u32,
    pub Format: DXGI_FORMAT,
    pub SampleDesc: DXGI_SAMPLE_DESC,
    pub Usage: D3D11_USAGE,
    pub BindFlags: u32,
    pub CPUAccessFlags: u32,
    pub MiscFlags: u32,
}
pub type D3D11_USAGE = i32;
pub const D3D11_USAGE_DEFAULT: D3D11_USAGE = 0;
pub const D3D11_USAGE_STAGING: D3D11_USAGE = 3;
pub type D3D_DRIVER_TYPE = i32;
pub const D3D_DRIVER_TYPE_HARDWARE: D3D_DRIVER_TYPE = 1;
pub type D3D_FEATURE_LEVEL = i32;
pub const D3D_FEATURE_LEVEL_11_0: D3D_FEATURE_LEVEL = 45056;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DECIMAL {
    pub wReserved: u16,
    pub Anonymous1: DECIMAL_0,
    pub Hi32: u32,
    pub Anonymous2: DECIMAL_1,
}
impl Default for DECIMAL {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union DECIMAL_0 {
    pub Anonymous: DECIMAL_0_0,
    pub signscale: u16,
}
impl Default for DECIMAL_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DECIMAL_0_0 {
    pub scale: u8,
    pub sign: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union DECIMAL_1 {
    pub Anonymous: DECIMAL_1_0,
    pub Lo64: u64,
}
impl Default for DECIMAL_1 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DECIMAL_1_0 {
    pub Lo32: u32,
    pub Mid32: u32,
}
pub type DEVICE_STATE = u32;
pub const DEVICE_STATE_ACTIVE: DEVICE_STATE = 1;
pub const DEVICE_STATE_DISABLED: DEVICE_STATE = 2;
pub const DEVICE_STATE_NOTPRESENT: DEVICE_STATE = 4;
pub const DEVICE_STATE_UNPLUGGED: DEVICE_STATE = 8;
pub type DXGI_FORMAT = i32;
pub const DXGI_FORMAT_B8G8R8A8_UNORM: DXGI_FORMAT = 87;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DXGI_SAMPLE_DESC {
    pub Count: u32,
    pub Quality: u32,
}
pub type EDataFlow = i32;
pub const EDataFlow_enum_count: EDataFlow = 3;
pub const ERROR_FILE_NOT_FOUND: WIN32_ERROR = 2;
pub type ERole = i32;
pub const ERole_enum_count: ERole = 3;
pub const E_ABORT: windows_core::HRESULT = windows_core::HRESULT(0x80004004_u32 as _);
pub const E_FAIL: windows_core::HRESULT = windows_core::HRESULT(0x80004005_u32 as _);
pub const E_INVALIDARG: windows_core::HRESULT = windows_core::HRESULT(0x80070057_u32 as _);
pub const E_NOTIMPL: windows_core::HRESULT = windows_core::HRESULT(0x80004001_u32 as _);
pub const E_POINTER: windows_core::HRESULT = windows_core::HRESULT(0x80004003_u32 as _);
pub type FILEOPERATION_FLAGS = u32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}
pub type FILE_MAP = u32;
pub const FILE_MAP_READ: FILE_MAP = 4;
pub const FOFX_ADDUNDORECORD: FILEOPERATION_FLAGS = 536870912;
pub const FOFX_EARLYFAILURE: FILEOPERATION_FLAGS = 1048576;
pub const FOFX_RECYCLEONDELETE: FILEOPERATION_FLAGS = 524288;
pub const FOF_ALLOWUNDO: FILEOPERATION_FLAGS = 64;
pub const FOF_NO_UI: FILEOPERATION_FLAGS = 1556;
pub const FileOperation: windows_core::GUID =
    windows_core::GUID::from_u128(0x3ad05575_8857_4850_9277_11b85bdb8e09);
pub type GLOBAL_ALLOC_FLAGS = u32;
pub const GMEM_MOVEABLE: GLOBAL_ALLOC_FLAGS = 2;
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HANDLE(pub *mut core::ffi::c_void);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HGLOBAL(pub *mut core::ffi::c_void);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HMODULE(pub *mut core::ffi::c_void);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HWND(pub *mut core::ffi::c_void);
windows_core::imp::define_interface!(
    IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceAsyncOperation_Vtbl,
    0x72a22d78_cde4_431d_b8cc_843a71199b6d
);
windows_core::imp::interface_hierarchy!(
    IActivateAudioInterfaceAsyncOperation,
    windows_core::IUnknown
);
impl IActivateAudioInterfaceAsyncOperation {
    pub unsafe fn GetActivateResult(
        &self,
        activateresult: *mut windows_core::HRESULT,
        activatedinterface: *mut Option<windows_core::IUnknown>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetActivateResult)(
                windows_core::Interface::as_raw(self),
                activateresult as _,
                core::mem::transmute(activatedinterface),
            )
        }
    }
}
#[repr(C)]
pub struct IActivateAudioInterfaceAsyncOperation_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetActivateResult: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut windows_core::HRESULT,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
pub trait IActivateAudioInterfaceAsyncOperation_Impl: windows_core::IUnknownImpl {
    fn GetActivateResult(
        &self,
        activateresult: *mut windows_core::HRESULT,
        activatedinterface: windows_core::OutRef<windows_core::IUnknown>,
    ) -> windows_core::Result<()>;
}
impl IActivateAudioInterfaceAsyncOperation_Vtbl {
    pub const fn new<Identity: IActivateAudioInterfaceAsyncOperation_Impl, const OFFSET: isize>(
    ) -> Self {
        unsafe extern "system" fn GetActivateResult<
            Identity: IActivateAudioInterfaceAsyncOperation_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            activateresult: *mut windows_core::HRESULT,
            activatedinterface: *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IActivateAudioInterfaceAsyncOperation_Impl::GetActivateResult(
                    this,
                    core::mem::transmute_copy(&activateresult),
                    core::mem::transmute_copy(&activatedinterface),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            GetActivateResult: GetActivateResult::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IActivateAudioInterfaceAsyncOperation as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IActivateAudioInterfaceAsyncOperation {}
windows_core::imp::define_interface!(
    IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Vtbl,
    0x41d949ab_9862_444a_80f6_c261334da5eb
);
windows_core::imp::interface_hierarchy!(
    IActivateAudioInterfaceCompletionHandler,
    windows_core::IUnknown
);
impl IActivateAudioInterfaceCompletionHandler {
    pub unsafe fn ActivateCompleted<P0>(&self, activateoperation: P0) -> windows_core::HRESULT
    where
        P0: windows_core::Param<IActivateAudioInterfaceAsyncOperation>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).ActivateCompleted)(
                windows_core::Interface::as_raw(self),
                activateoperation.param().abi(),
            )
        }
    }
}
#[repr(C)]
pub struct IActivateAudioInterfaceCompletionHandler_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub ActivateCompleted: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
pub trait IActivateAudioInterfaceCompletionHandler_Impl: windows_core::IUnknownImpl {
    fn ActivateCompleted(
        &self,
        activateoperation: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows_core::Result<()>;
}
impl IActivateAudioInterfaceCompletionHandler_Vtbl {
    pub const fn new<
        Identity: IActivateAudioInterfaceCompletionHandler_Impl,
        const OFFSET: isize,
    >() -> Self {
        unsafe extern "system" fn ActivateCompleted<
            Identity: IActivateAudioInterfaceCompletionHandler_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            activateoperation: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IActivateAudioInterfaceCompletionHandler_Impl::ActivateCompleted(
                    this,
                    core::mem::transmute_copy(&activateoperation),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            ActivateCompleted: ActivateCompleted::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IActivateAudioInterfaceCompletionHandler as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IActivateAudioInterfaceCompletionHandler {}
windows_core::imp::define_interface!(
    IAgileObject,
    IAgileObject_Vtbl,
    0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90
);
windows_core::imp::interface_hierarchy!(IAgileObject, windows_core::IUnknown);
#[repr(C)]
pub struct IAgileObject_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
}
pub trait IAgileObject_Impl: windows_core::IUnknownImpl {}
impl IAgileObject_Vtbl {
    pub const fn new<Identity: IAgileObject_Impl, const OFFSET: isize>() -> Self {
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IAgileObject as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IAgileObject {}
windows_core::imp::define_interface!(
    IAudioCaptureClient,
    IAudioCaptureClient_Vtbl,
    0xc8adbd64_e71e_48a0_a4de_185c395cd317
);
windows_core::imp::interface_hierarchy!(IAudioCaptureClient, windows_core::IUnknown);
impl IAudioCaptureClient {
    pub unsafe fn GetBuffer(
        &self,
        ppdata: *mut *mut u8,
        pnumframestoread: *mut u32,
        pdwflags: *mut u32,
        pu64deviceposition: Option<*mut u64>,
        pu64qpcposition: Option<*mut u64>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetBuffer)(
                windows_core::Interface::as_raw(self),
                ppdata as _,
                pnumframestoread as _,
                pdwflags as _,
                pu64deviceposition.unwrap_or(core::mem::zeroed()) as _,
                pu64qpcposition.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn ReleaseBuffer(&self, numframesread: u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).ReleaseBuffer)(
                windows_core::Interface::as_raw(self),
                numframesread,
            )
        }
    }
    pub unsafe fn GetNextPacketSize(&self) -> windows_core::Result<u32> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetNextPacketSize)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IAudioCaptureClient_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetBuffer: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut u8,
        *mut u32,
        *mut u32,
        *mut u64,
        *mut u64,
    ) -> windows_core::HRESULT,
    pub ReleaseBuffer:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> windows_core::HRESULT,
    pub GetNextPacketSize:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32) -> windows_core::HRESULT,
}
pub trait IAudioCaptureClient_Impl: windows_core::IUnknownImpl {
    fn GetBuffer(
        &self,
        ppdata: *mut *mut u8,
        pnumframestoread: *mut u32,
        pdwflags: *mut u32,
        pu64deviceposition: *mut u64,
        pu64qpcposition: *mut u64,
    ) -> windows_core::Result<()>;
    fn ReleaseBuffer(&self, numframesread: u32) -> windows_core::Result<()>;
    fn GetNextPacketSize(&self) -> windows_core::Result<u32>;
}
impl IAudioCaptureClient_Vtbl {
    pub const fn new<Identity: IAudioCaptureClient_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn GetBuffer<
            Identity: IAudioCaptureClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            ppdata: *mut *mut u8,
            pnumframestoread: *mut u32,
            pdwflags: *mut u32,
            pu64deviceposition: *mut u64,
            pu64qpcposition: *mut u64,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IAudioCaptureClient_Impl::GetBuffer(
                    this,
                    core::mem::transmute_copy(&ppdata),
                    core::mem::transmute_copy(&pnumframestoread),
                    core::mem::transmute_copy(&pdwflags),
                    core::mem::transmute_copy(&pu64deviceposition),
                    core::mem::transmute_copy(&pu64qpcposition),
                )
                .into()
            }
        }
        unsafe extern "system" fn ReleaseBuffer<
            Identity: IAudioCaptureClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            numframesread: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IAudioCaptureClient_Impl::ReleaseBuffer(
                    this,
                    core::mem::transmute_copy(&numframesread),
                )
                .into()
            }
        }
        unsafe extern "system" fn GetNextPacketSize<
            Identity: IAudioCaptureClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pnumframesinnextpacket: *mut u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                match IAudioCaptureClient_Impl::GetNextPacketSize(this) {
                    Ok(ok__) => {
                        pnumframesinnextpacket.write(ok__);
                        windows_core::HRESULT(0)
                    }
                    Err(err) => err.into(),
                }
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            GetBuffer: GetBuffer::<Identity, OFFSET>,
            ReleaseBuffer: ReleaseBuffer::<Identity, OFFSET>,
            GetNextPacketSize: GetNextPacketSize::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IAudioCaptureClient as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IAudioCaptureClient {}
windows_core::imp::define_interface!(
    IAudioClient,
    IAudioClient_Vtbl,
    0x1cb9ad4c_dbfa_4c32_b178_c2f568a703b2
);
windows_core::imp::interface_hierarchy!(IAudioClient, windows_core::IUnknown);
impl IAudioClient {
    pub unsafe fn Initialize(
        &self,
        sharemode: AUDCLNT_SHAREMODE,
        streamflags: u32,
        hnsbufferduration: i64,
        hnsperiodicity: i64,
        pformat: *const WAVEFORMATEX,
        audiosessionguid: Option<*const windows_core::GUID>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Initialize)(
                windows_core::Interface::as_raw(self),
                sharemode,
                streamflags,
                hnsbufferduration,
                hnsperiodicity,
                pformat,
                audiosessionguid.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn Start(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Start)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn Stop(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Stop)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetEventHandle(&self, eventhandle: HANDLE) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetEventHandle)(
                windows_core::Interface::as_raw(self),
                eventhandle,
            )
        }
    }
    pub unsafe fn GetService<T>(&self) -> windows_core::Result<T>
    where
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).GetService)(
                windows_core::Interface::as_raw(self),
                &T::IID,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IAudioClient_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub Initialize: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        AUDCLNT_SHAREMODE,
        u32,
        i64,
        i64,
        *const WAVEFORMATEX,
        *const windows_core::GUID,
    ) -> windows_core::HRESULT,
    GetBufferSize: usize,
    GetStreamLatency: usize,
    GetCurrentPadding: usize,
    IsFormatSupported: usize,
    GetMixFormat: usize,
    GetDevicePeriod: usize,
    pub Start: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub Stop: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    Reset: usize,
    pub SetEventHandle:
        unsafe extern "system" fn(*mut core::ffi::c_void, HANDLE) -> windows_core::HRESULT,
    pub GetService: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IAudioClient {}
windows_core::imp::define_interface!(
    IAudioFormatEnumerator,
    IAudioFormatEnumerator_Vtbl,
    0xdcdaa858_895a_4a22_a5eb_67bda506096d
);
windows_core::imp::interface_hierarchy!(IAudioFormatEnumerator, windows_core::IUnknown);
impl IAudioFormatEnumerator {
    pub unsafe fn GetFormat(&self, index: u32) -> windows_core::Result<*mut WAVEFORMATEX> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetFormat)(
                windows_core::Interface::as_raw(self),
                index,
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IAudioFormatEnumerator_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetCount: usize,
    pub GetFormat: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut *mut WAVEFORMATEX,
    ) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IAudioFormatEnumerator {}
windows_core::imp::define_interface!(
    IAudioSourceProvider,
    IAudioSourceProvider_Vtbl,
    0xebbaf249_afc2_4582_91c6_b60df2e84954
);
windows_core::imp::interface_hierarchy!(IAudioSourceProvider, windows_core::IUnknown);
impl IAudioSourceProvider {
    pub unsafe fn ProvideInput(
        &self,
        dwsamplecount: u32,
        pdwchannelcount: *mut u32,
        pinterleavedaudiodata: Option<*mut f32>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).ProvideInput)(
                windows_core::Interface::as_raw(self),
                dwsamplecount,
                pdwchannelcount as _,
                pinterleavedaudiodata.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
}
#[repr(C)]
pub struct IAudioSourceProvider_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub ProvideInput: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut u32,
        *mut f32,
    ) -> windows_core::HRESULT,
}
pub trait IAudioSourceProvider_Impl: windows_core::IUnknownImpl {
    fn ProvideInput(
        &self,
        dwsamplecount: u32,
        pdwchannelcount: *mut u32,
        pinterleavedaudiodata: *mut f32,
    ) -> windows_core::Result<()>;
}
impl IAudioSourceProvider_Vtbl {
    pub const fn new<Identity: IAudioSourceProvider_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn ProvideInput<
            Identity: IAudioSourceProvider_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwsamplecount: u32,
            pdwchannelcount: *mut u32,
            pinterleavedaudiodata: *mut f32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IAudioSourceProvider_Impl::ProvideInput(
                    this,
                    core::mem::transmute_copy(&dwsamplecount),
                    core::mem::transmute_copy(&pdwchannelcount),
                    core::mem::transmute_copy(&pinterleavedaudiodata),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            ProvideInput: ProvideInput::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IAudioSourceProvider as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IAudioSourceProvider {}
windows_core::imp::define_interface!(
    IBindCtx,
    IBindCtx_Vtbl,
    0x0000000e_0000_0000_c000_000000000046
);
windows_core::imp::interface_hierarchy!(IBindCtx, windows_core::IUnknown);
#[repr(C)]
pub struct IBindCtx_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    RegisterObjectBound: usize,
    RevokeObjectBound: usize,
    ReleaseBoundObjects: usize,
    SetBindOptions: usize,
    GetBindOptions: usize,
    GetRunningObjectTable: usize,
    RegisterObjectParam: usize,
    GetObjectParam: usize,
    EnumObjectParam: usize,
    RevokeObjectParam: usize,
}
impl windows_core::RuntimeName for IBindCtx {}
windows_core::imp::define_interface!(
    IClassFactory,
    IClassFactory_Vtbl,
    0x00000001_0000_0000_c000_000000000046
);
windows_core::imp::interface_hierarchy!(IClassFactory, windows_core::IUnknown);
impl IClassFactory {
    pub unsafe fn CreateInstance<P0, T>(&self, punkouter: P0) -> windows_core::Result<T>
    where
        P0: windows_core::Param<windows_core::IUnknown>,
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).CreateInstance)(
                windows_core::Interface::as_raw(self),
                punkouter.param().abi(),
                &T::IID,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn LockServer(&self, flock: bool) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).LockServer)(
                windows_core::Interface::as_raw(self),
                flock.into(),
            )
        }
    }
}
#[repr(C)]
pub struct IClassFactory_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub CreateInstance: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub LockServer: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::BOOL,
    ) -> windows_core::HRESULT,
}
pub trait IClassFactory_Impl: windows_core::IUnknownImpl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<windows_core::IUnknown>,
        riid: *const windows_core::GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()>;
    fn LockServer(&self, flock: windows_core::BOOL) -> windows_core::Result<()>;
}
impl IClassFactory_Vtbl {
    pub const fn new<Identity: IClassFactory_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn CreateInstance<
            Identity: IClassFactory_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            punkouter: *mut core::ffi::c_void,
            riid: *const windows_core::GUID,
            ppvobject: *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IClassFactory_Impl::CreateInstance(
                    this,
                    core::mem::transmute_copy(&punkouter),
                    core::mem::transmute_copy(&riid),
                    core::mem::transmute_copy(&ppvobject),
                )
                .into()
            }
        }
        unsafe extern "system" fn LockServer<Identity: IClassFactory_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            flock: windows_core::BOOL,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IClassFactory_Impl::LockServer(this, core::mem::transmute_copy(&flock)).into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            CreateInstance: CreateInstance::<Identity, OFFSET>,
            LockServer: LockServer::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IClassFactory as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IClassFactory {}
windows_core::imp::define_interface!(
    ID3D11Device,
    ID3D11Device_Vtbl,
    0xdb6f6ddb_ac77_4e88_8253_819df9bbf140
);
windows_core::imp::interface_hierarchy!(ID3D11Device, windows_core::IUnknown);
impl ID3D11Device {
    pub unsafe fn CreateTexture2D(
        &self,
        pdesc: *const D3D11_TEXTURE2D_DESC,
        pinitialdata: Option<*const D3D11_SUBRESOURCE_DATA>,
        pptexture2d: Option<*mut Option<ID3D11Texture2D>>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).CreateTexture2D)(
                windows_core::Interface::as_raw(self),
                pdesc,
                pinitialdata.unwrap_or(core::mem::zeroed()) as _,
                pptexture2d.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn GetImmediateContext(&self) -> windows_core::Result<ID3D11DeviceContext> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetImmediateContext)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            );
            windows_core::imp::Type::from_abi(result__)
        }
    }
}
#[repr(C)]
pub struct ID3D11Device_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    CreateBuffer: usize,
    CreateTexture1D: usize,
    pub CreateTexture2D: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const D3D11_TEXTURE2D_DESC,
        *const D3D11_SUBRESOURCE_DATA,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    CreateTexture3D: usize,
    CreateShaderResourceView: usize,
    CreateUnorderedAccessView: usize,
    CreateRenderTargetView: usize,
    CreateDepthStencilView: usize,
    CreateInputLayout: usize,
    CreateVertexShader: usize,
    CreateGeometryShader: usize,
    CreateGeometryShaderWithStreamOutput: usize,
    CreatePixelShader: usize,
    CreateHullShader: usize,
    CreateDomainShader: usize,
    CreateComputeShader: usize,
    CreateClassLinkage: usize,
    CreateBlendState: usize,
    CreateDepthStencilState: usize,
    CreateRasterizerState: usize,
    CreateSamplerState: usize,
    CreateQuery: usize,
    CreatePredicate: usize,
    CreateCounter: usize,
    CreateDeferredContext: usize,
    OpenSharedResource: usize,
    CheckFormatSupport: usize,
    CheckMultisampleQualityLevels: usize,
    CheckCounterInfo: usize,
    CheckCounter: usize,
    CheckFeatureSupport: usize,
    GetPrivateData: usize,
    SetPrivateData: usize,
    SetPrivateDataInterface: usize,
    GetFeatureLevel: usize,
    GetCreationFlags: usize,
    GetDeviceRemovedReason: usize,
    pub GetImmediateContext:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void),
    SetExceptionMode: usize,
    GetExceptionMode: usize,
}
impl windows_core::RuntimeName for ID3D11Device {}
windows_core::imp::define_interface!(
    ID3D11DeviceChild,
    ID3D11DeviceChild_Vtbl,
    0x1841e5c8_16b0_489b_bcc8_44cfb0d5deae
);
windows_core::imp::interface_hierarchy!(ID3D11DeviceChild, windows_core::IUnknown);
#[repr(C)]
pub struct ID3D11DeviceChild_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetDevice: usize,
    GetPrivateData: usize,
    SetPrivateData: usize,
    SetPrivateDataInterface: usize,
}
impl windows_core::RuntimeName for ID3D11DeviceChild {}
windows_core::imp::define_interface!(
    ID3D11DeviceContext,
    ID3D11DeviceContext_Vtbl,
    0xc0bfa96c_e089_44fb_8eaf_26f8796190da
);
impl core::ops::Deref for ID3D11DeviceContext {
    type Target = ID3D11DeviceChild;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(
    ID3D11DeviceContext,
    windows_core::IUnknown,
    ID3D11DeviceChild
);
impl ID3D11DeviceContext {
    pub unsafe fn Map<P0>(
        &self,
        presource: P0,
        subresource: u32,
        maptype: D3D11_MAP,
        mapflags: u32,
        pmappedresource: Option<*mut D3D11_MAPPED_SUBRESOURCE>,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<ID3D11Resource>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).Map)(
                windows_core::Interface::as_raw(self),
                presource.param().abi(),
                subresource,
                maptype,
                mapflags,
                pmappedresource.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn Unmap<P0>(&self, presource: P0, subresource: u32)
    where
        P0: windows_core::Param<ID3D11Resource>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).Unmap)(
                windows_core::Interface::as_raw(self),
                presource.param().abi(),
                subresource,
            );
        }
    }
    pub unsafe fn CopyResource<P0, P1>(&self, pdstresource: P0, psrcresource: P1)
    where
        P0: windows_core::Param<ID3D11Resource>,
        P1: windows_core::Param<ID3D11Resource>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).CopyResource)(
                windows_core::Interface::as_raw(self),
                pdstresource.param().abi(),
                psrcresource.param().abi(),
            );
        }
    }
    pub unsafe fn Flush(&self) {
        unsafe {
            (windows_core::Interface::vtable(self).Flush)(windows_core::Interface::as_raw(self));
        }
    }
}
#[repr(C)]
pub struct ID3D11DeviceContext_Vtbl {
    pub base__: ID3D11DeviceChild_Vtbl,
    VSSetConstantBuffers: usize,
    PSSetShaderResources: usize,
    PSSetShader: usize,
    PSSetSamplers: usize,
    VSSetShader: usize,
    DrawIndexed: usize,
    Draw: usize,
    pub Map: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        u32,
        D3D11_MAP,
        u32,
        *mut D3D11_MAPPED_SUBRESOURCE,
    ) -> windows_core::HRESULT,
    pub Unmap: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void, u32),
    PSSetConstantBuffers: usize,
    IASetInputLayout: usize,
    IASetVertexBuffers: usize,
    IASetIndexBuffer: usize,
    DrawIndexedInstanced: usize,
    DrawInstanced: usize,
    GSSetConstantBuffers: usize,
    GSSetShader: usize,
    IASetPrimitiveTopology: usize,
    VSSetShaderResources: usize,
    VSSetSamplers: usize,
    Begin: usize,
    End: usize,
    GetData: usize,
    SetPredication: usize,
    GSSetShaderResources: usize,
    GSSetSamplers: usize,
    OMSetRenderTargets: usize,
    OMSetRenderTargetsAndUnorderedAccessViews: usize,
    OMSetBlendState: usize,
    OMSetDepthStencilState: usize,
    SOSetTargets: usize,
    DrawAuto: usize,
    DrawIndexedInstancedIndirect: usize,
    DrawInstancedIndirect: usize,
    Dispatch: usize,
    DispatchIndirect: usize,
    RSSetState: usize,
    RSSetViewports: usize,
    RSSetScissorRects: usize,
    CopySubresourceRegion: usize,
    pub CopyResource: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ),
    UpdateSubresource: usize,
    CopyStructureCount: usize,
    ClearRenderTargetView: usize,
    ClearUnorderedAccessViewUint: usize,
    ClearUnorderedAccessViewFloat: usize,
    ClearDepthStencilView: usize,
    GenerateMips: usize,
    SetResourceMinLOD: usize,
    GetResourceMinLOD: usize,
    ResolveSubresource: usize,
    ExecuteCommandList: usize,
    HSSetShaderResources: usize,
    HSSetShader: usize,
    HSSetSamplers: usize,
    HSSetConstantBuffers: usize,
    DSSetShaderResources: usize,
    DSSetShader: usize,
    DSSetSamplers: usize,
    DSSetConstantBuffers: usize,
    CSSetShaderResources: usize,
    CSSetUnorderedAccessViews: usize,
    CSSetShader: usize,
    CSSetSamplers: usize,
    CSSetConstantBuffers: usize,
    VSGetConstantBuffers: usize,
    PSGetShaderResources: usize,
    PSGetShader: usize,
    PSGetSamplers: usize,
    VSGetShader: usize,
    PSGetConstantBuffers: usize,
    IAGetInputLayout: usize,
    IAGetVertexBuffers: usize,
    IAGetIndexBuffer: usize,
    GSGetConstantBuffers: usize,
    GSGetShader: usize,
    IAGetPrimitiveTopology: usize,
    VSGetShaderResources: usize,
    VSGetSamplers: usize,
    GetPredication: usize,
    GSGetShaderResources: usize,
    GSGetSamplers: usize,
    OMGetRenderTargets: usize,
    OMGetRenderTargetsAndUnorderedAccessViews: usize,
    OMGetBlendState: usize,
    OMGetDepthStencilState: usize,
    SOGetTargets: usize,
    RSGetState: usize,
    RSGetViewports: usize,
    RSGetScissorRects: usize,
    HSGetShaderResources: usize,
    HSGetShader: usize,
    HSGetSamplers: usize,
    HSGetConstantBuffers: usize,
    DSGetShaderResources: usize,
    DSGetShader: usize,
    DSGetSamplers: usize,
    DSGetConstantBuffers: usize,
    CSGetShaderResources: usize,
    CSGetUnorderedAccessViews: usize,
    CSGetShader: usize,
    CSGetSamplers: usize,
    CSGetConstantBuffers: usize,
    ClearState: usize,
    pub Flush: unsafe extern "system" fn(*mut core::ffi::c_void),
    GetType: usize,
    GetContextFlags: usize,
    FinishCommandList: usize,
}
impl windows_core::RuntimeName for ID3D11DeviceContext {}
windows_core::imp::define_interface!(
    ID3D11Multithread,
    ID3D11Multithread_Vtbl,
    0x9b7e4e00_342c_4106_a19f_4f2704f689f0
);
windows_core::imp::interface_hierarchy!(ID3D11Multithread, windows_core::IUnknown);
impl ID3D11Multithread {
    pub unsafe fn SetMultithreadProtected(&self, bmtprotect: bool) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).SetMultithreadProtected)(
                windows_core::Interface::as_raw(self),
                bmtprotect.into(),
            )
        }
    }
}
#[repr(C)]
pub struct ID3D11Multithread_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    Enter: usize,
    Leave: usize,
    pub SetMultithreadProtected:
        unsafe extern "system" fn(*mut core::ffi::c_void, windows_core::BOOL) -> windows_core::BOOL,
    GetMultithreadProtected: usize,
}
impl windows_core::RuntimeName for ID3D11Multithread {}
windows_core::imp::define_interface!(
    ID3D11Resource,
    ID3D11Resource_Vtbl,
    0xdc8e63f3_d12b_4952_b47b_5e45026a862d
);
impl core::ops::Deref for ID3D11Resource {
    type Target = ID3D11DeviceChild;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(ID3D11Resource, windows_core::IUnknown, ID3D11DeviceChild);
#[repr(C)]
pub struct ID3D11Resource_Vtbl {
    pub base__: ID3D11DeviceChild_Vtbl,
    GetType: usize,
    SetEvictionPriority: usize,
    GetEvictionPriority: usize,
}
impl windows_core::RuntimeName for ID3D11Resource {}
windows_core::imp::define_interface!(
    ID3D11Texture2D,
    ID3D11Texture2D_Vtbl,
    0x6f15aaf2_d208_4e89_9ab4_489535d34f9c
);
impl core::ops::Deref for ID3D11Texture2D {
    type Target = ID3D11Resource;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(
    ID3D11Texture2D,
    windows_core::IUnknown,
    ID3D11DeviceChild,
    ID3D11Resource
);
#[repr(C)]
pub struct ID3D11Texture2D_Vtbl {
    pub base__: ID3D11Resource_Vtbl,
    GetDesc: usize,
}
impl windows_core::RuntimeName for ID3D11Texture2D {}
windows_core::imp::define_interface!(
    IDXGIAdapter,
    IDXGIAdapter_Vtbl,
    0x2411e7e1_12ac_4ccf_bd14_9798e8534dc0
);
impl core::ops::Deref for IDXGIAdapter {
    type Target = IDXGIObject;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(IDXGIAdapter, windows_core::IUnknown, IDXGIObject);
#[repr(C)]
pub struct IDXGIAdapter_Vtbl {
    pub base__: IDXGIObject_Vtbl,
    EnumOutputs: usize,
    GetDesc: usize,
    CheckInterfaceSupport: usize,
}
impl windows_core::RuntimeName for IDXGIAdapter {}
windows_core::imp::define_interface!(
    IDXGIObject,
    IDXGIObject_Vtbl,
    0xaec22fb8_76f3_4639_9be0_28eb43a67a2e
);
windows_core::imp::interface_hierarchy!(IDXGIObject, windows_core::IUnknown);
#[repr(C)]
pub struct IDXGIObject_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    SetPrivateData: usize,
    SetPrivateDataInterface: usize,
    GetPrivateData: usize,
    GetParent: usize,
}
impl windows_core::RuntimeName for IDXGIObject {}
windows_core::imp::define_interface!(
    IDataTransferManagerInterop,
    IDataTransferManagerInterop_Vtbl,
    0x3a3dcd6c_3eab_43dc_bcde_45671ce800c8
);
windows_core::imp::interface_hierarchy!(IDataTransferManagerInterop, windows_core::IUnknown);
impl IDataTransferManagerInterop {
    pub unsafe fn GetForWindow<T>(&self, appwindow: HWND) -> windows_core::Result<T>
    where
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).GetForWindow)(
                windows_core::Interface::as_raw(self),
                appwindow,
                &T::IID,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn ShowShareUIForWindow(&self, appwindow: HWND) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).ShowShareUIForWindow)(
                windows_core::Interface::as_raw(self),
                appwindow,
            )
        }
    }
}
#[repr(C)]
pub struct IDataTransferManagerInterop_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetForWindow: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        HWND,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub ShowShareUIForWindow:
        unsafe extern "system" fn(*mut core::ffi::c_void, HWND) -> windows_core::HRESULT,
}
pub trait IDataTransferManagerInterop_Impl: windows_core::IUnknownImpl {
    fn GetForWindow(
        &self,
        appwindow: HWND,
        riid: *const windows_core::GUID,
        datatransfermanager: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()>;
    fn ShowShareUIForWindow(&self, appwindow: HWND) -> windows_core::Result<()>;
}
impl IDataTransferManagerInterop_Vtbl {
    pub const fn new<Identity: IDataTransferManagerInterop_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn GetForWindow<
            Identity: IDataTransferManagerInterop_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            appwindow: HWND,
            riid: *const windows_core::GUID,
            datatransfermanager: *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IDataTransferManagerInterop_Impl::GetForWindow(
                    this,
                    core::mem::transmute_copy(&appwindow),
                    core::mem::transmute_copy(&riid),
                    core::mem::transmute_copy(&datatransfermanager),
                )
                .into()
            }
        }
        unsafe extern "system" fn ShowShareUIForWindow<
            Identity: IDataTransferManagerInterop_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            appwindow: HWND,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IDataTransferManagerInterop_Impl::ShowShareUIForWindow(
                    this,
                    core::mem::transmute_copy(&appwindow),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            GetForWindow: GetForWindow::<Identity, OFFSET>,
            ShowShareUIForWindow: ShowShareUIForWindow::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IDataTransferManagerInterop as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IDataTransferManagerInterop {}
windows_core::imp::define_interface!(
    IDispatch,
    IDispatch_Vtbl,
    0x00020400_0000_0000_c000_000000000046
);
windows_core::imp::interface_hierarchy!(IDispatch, windows_core::IUnknown);
#[repr(C)]
pub struct IDispatch_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetTypeInfoCount: usize,
    GetTypeInfo: usize,
    GetIDsOfNames: usize,
    Invoke: usize,
}
impl windows_core::RuntimeName for IDispatch {}
windows_core::imp::define_interface!(
    IFileOperation,
    IFileOperation_Vtbl,
    0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8
);
windows_core::imp::interface_hierarchy!(IFileOperation, windows_core::IUnknown);
impl IFileOperation {
    pub unsafe fn SetOperationFlags(
        &self,
        dwoperationflags: FILEOPERATION_FLAGS,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetOperationFlags)(
                windows_core::Interface::as_raw(self),
                dwoperationflags,
            )
        }
    }
    pub unsafe fn DeleteItem<P0, P1>(&self, psiitem: P0, pfopsitem: P1) -> windows_core::HRESULT
    where
        P0: windows_core::Param<IShellItem>,
        P1: windows_core::Param<IFileOperationProgressSink>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).DeleteItem)(
                windows_core::Interface::as_raw(self),
                psiitem.param().abi(),
                pfopsitem.param().abi(),
            )
        }
    }
    pub unsafe fn PerformOperations(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).PerformOperations)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
    pub unsafe fn GetAnyOperationsAborted(&self) -> windows_core::Result<windows_core::BOOL> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetAnyOperationsAborted)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IFileOperation_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    Advise: usize,
    Unadvise: usize,
    pub SetOperationFlags: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        FILEOPERATION_FLAGS,
    ) -> windows_core::HRESULT,
    SetProgressMessage: usize,
    SetProgressDialog: usize,
    SetProperties: usize,
    SetOwnerWindow: usize,
    ApplyPropertiesToItem: usize,
    ApplyPropertiesToItems: usize,
    RenameItem: usize,
    RenameItems: usize,
    MoveItem: usize,
    MoveItems: usize,
    CopyItem: usize,
    CopyItems: usize,
    pub DeleteItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    DeleteItems: usize,
    NewItem: usize,
    pub PerformOperations:
        unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub GetAnyOperationsAborted: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut windows_core::BOOL,
    ) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IFileOperation {}
windows_core::imp::define_interface!(
    IFileOperationProgressSink,
    IFileOperationProgressSink_Vtbl,
    0x04b0f1a7_9490_44bc_96e1_4296a31252e2
);
windows_core::imp::interface_hierarchy!(IFileOperationProgressSink, windows_core::IUnknown);
impl IFileOperationProgressSink {
    pub unsafe fn StartOperations(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).StartOperations)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
    pub unsafe fn FinishOperations(
        &self,
        hrresult: windows_core::HRESULT,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).FinishOperations)(
                windows_core::Interface::as_raw(self),
                hrresult,
            )
        }
    }
    pub unsafe fn PreRenameItem<P1, P2>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psznewname: P2,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PreRenameItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psznewname.param().abi(),
            )
        }
    }
    pub unsafe fn PostRenameItem<P1, P2, P4>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psznewname: P2,
        hrrename: windows_core::HRESULT,
        psinewlycreated: P4,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<windows_core::PCWSTR>,
        P4: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PostRenameItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psznewname.param().abi(),
                hrrename,
                psinewlycreated.param().abi(),
            )
        }
    }
    pub unsafe fn PreMoveItem<P1, P2, P3>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psidestinationfolder: P2,
        psznewname: P3,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<IShellItem>,
        P3: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PreMoveItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
            )
        }
    }
    pub unsafe fn PostMoveItem<P1, P2, P3, P5>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psidestinationfolder: P2,
        psznewname: P3,
        hrmove: windows_core::HRESULT,
        psinewlycreated: P5,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<IShellItem>,
        P3: windows_core::Param<windows_core::PCWSTR>,
        P5: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PostMoveItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
                hrmove,
                psinewlycreated.param().abi(),
            )
        }
    }
    pub unsafe fn PreCopyItem<P1, P2, P3>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psidestinationfolder: P2,
        psznewname: P3,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<IShellItem>,
        P3: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PreCopyItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
            )
        }
    }
    pub unsafe fn PostCopyItem<P1, P2, P3, P5>(
        &self,
        dwflags: u32,
        psiitem: P1,
        psidestinationfolder: P2,
        psznewname: P3,
        hrcopy: windows_core::HRESULT,
        psinewlycreated: P5,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<IShellItem>,
        P3: windows_core::Param<windows_core::PCWSTR>,
        P5: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PostCopyItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
                hrcopy,
                psinewlycreated.param().abi(),
            )
        }
    }
    pub unsafe fn PreDeleteItem<P1>(&self, dwflags: u32, psiitem: P1) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PreDeleteItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
            )
        }
    }
    pub unsafe fn PostDeleteItem<P1, P3>(
        &self,
        dwflags: u32,
        psiitem: P1,
        hrdelete: windows_core::HRESULT,
        psinewlycreated: P3,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P3: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PostDeleteItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psiitem.param().abi(),
                hrdelete,
                psinewlycreated.param().abi(),
            )
        }
    }
    pub unsafe fn PreNewItem<P1, P2>(
        &self,
        dwflags: u32,
        psidestinationfolder: P1,
        psznewname: P2,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PreNewItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
            )
        }
    }
    pub unsafe fn PostNewItem<P1, P2, P3, P6>(
        &self,
        dwflags: u32,
        psidestinationfolder: P1,
        psznewname: P2,
        psztemplatename: P3,
        dwfileattributes: u32,
        hrnew: windows_core::HRESULT,
        psinewitem: P6,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IShellItem>,
        P2: windows_core::Param<windows_core::PCWSTR>,
        P3: windows_core::Param<windows_core::PCWSTR>,
        P6: windows_core::Param<IShellItem>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).PostNewItem)(
                windows_core::Interface::as_raw(self),
                dwflags,
                psidestinationfolder.param().abi(),
                psznewname.param().abi(),
                psztemplatename.param().abi(),
                dwfileattributes,
                hrnew,
                psinewitem.param().abi(),
            )
        }
    }
    pub unsafe fn UpdateProgress(&self, iworktotal: u32, iworksofar: u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).UpdateProgress)(
                windows_core::Interface::as_raw(self),
                iworktotal,
                iworksofar,
            )
        }
    }
    pub unsafe fn ResetTimer(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).ResetTimer)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn PauseTimer(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).PauseTimer)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn ResumeTimer(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).ResumeTimer)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
}
#[repr(C)]
pub struct IFileOperationProgressSink_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub StartOperations: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub FinishOperations: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::HRESULT,
    ) -> windows_core::HRESULT,
    pub PreRenameItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub PostRenameItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        windows_core::HRESULT,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub PreMoveItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub PostMoveItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        windows_core::HRESULT,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub PreCopyItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub PostCopyItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        windows_core::HRESULT,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub PreDeleteItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub PostDeleteItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        windows_core::HRESULT,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub PreNewItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub PostNewItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        windows_core::PCWSTR,
        u32,
        windows_core::HRESULT,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub UpdateProgress:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32, u32) -> windows_core::HRESULT,
    pub ResetTimer: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub PauseTimer: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub ResumeTimer: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
}
pub trait IFileOperationProgressSink_Impl: windows_core::IUnknownImpl {
    fn StartOperations(&self) -> windows_core::Result<()>;
    fn FinishOperations(&self, hrresult: windows_core::HRESULT) -> windows_core::Result<()>;
    fn PreRenameItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
    ) -> windows_core::Result<()>;
    fn PostRenameItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
        hrrename: windows_core::HRESULT,
        psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn PreMoveItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
    ) -> windows_core::Result<()>;
    fn PostMoveItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
        hrmove: windows_core::HRESULT,
        psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn PreCopyItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
    ) -> windows_core::Result<()>;
    fn PostCopyItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
        hrcopy: windows_core::HRESULT,
        psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn PreDeleteItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn PostDeleteItem(
        &self,
        dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        hrdelete: windows_core::HRESULT,
        psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn PreNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
    ) -> windows_core::Result<()>;
    fn PostNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &windows_core::PCWSTR,
        psztemplatename: &windows_core::PCWSTR,
        dwfileattributes: u32,
        hrnew: windows_core::HRESULT,
        psinewitem: windows_core::Ref<IShellItem>,
    ) -> windows_core::Result<()>;
    fn UpdateProgress(&self, iworktotal: u32, iworksofar: u32) -> windows_core::Result<()>;
    fn ResetTimer(&self) -> windows_core::Result<()>;
    fn PauseTimer(&self) -> windows_core::Result<()>;
    fn ResumeTimer(&self) -> windows_core::Result<()>;
}
impl IFileOperationProgressSink_Vtbl {
    pub const fn new<Identity: IFileOperationProgressSink_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn StartOperations<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::StartOperations(this).into()
            }
        }
        unsafe extern "system" fn FinishOperations<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            hrresult: windows_core::HRESULT,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::FinishOperations(
                    this,
                    core::mem::transmute_copy(&hrresult),
                )
                .into()
            }
        }
        unsafe extern "system" fn PreRenameItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PreRenameItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute(&psznewname),
                )
                .into()
            }
        }
        unsafe extern "system" fn PostRenameItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
            hrrename: windows_core::HRESULT,
            psinewlycreated: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PostRenameItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute(&psznewname),
                    core::mem::transmute_copy(&hrrename),
                    core::mem::transmute_copy(&psinewlycreated),
                )
                .into()
            }
        }
        unsafe extern "system" fn PreMoveItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PreMoveItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                )
                .into()
            }
        }
        unsafe extern "system" fn PostMoveItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
            hrmove: windows_core::HRESULT,
            psinewlycreated: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PostMoveItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                    core::mem::transmute_copy(&hrmove),
                    core::mem::transmute_copy(&psinewlycreated),
                )
                .into()
            }
        }
        unsafe extern "system" fn PreCopyItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PreCopyItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                )
                .into()
            }
        }
        unsafe extern "system" fn PostCopyItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
            hrcopy: windows_core::HRESULT,
            psinewlycreated: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PostCopyItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                    core::mem::transmute_copy(&hrcopy),
                    core::mem::transmute_copy(&psinewlycreated),
                )
                .into()
            }
        }
        unsafe extern "system" fn PreDeleteItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PreDeleteItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                )
                .into()
            }
        }
        unsafe extern "system" fn PostDeleteItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psiitem: *mut core::ffi::c_void,
            hrdelete: windows_core::HRESULT,
            psinewlycreated: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PostDeleteItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psiitem),
                    core::mem::transmute_copy(&hrdelete),
                    core::mem::transmute_copy(&psinewlycreated),
                )
                .into()
            }
        }
        unsafe extern "system" fn PreNewItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PreNewItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                )
                .into()
            }
        }
        unsafe extern "system" fn PostNewItem<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            dwflags: u32,
            psidestinationfolder: *mut core::ffi::c_void,
            psznewname: windows_core::PCWSTR,
            psztemplatename: windows_core::PCWSTR,
            dwfileattributes: u32,
            hrnew: windows_core::HRESULT,
            psinewitem: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PostNewItem(
                    this,
                    core::mem::transmute_copy(&dwflags),
                    core::mem::transmute_copy(&psidestinationfolder),
                    core::mem::transmute(&psznewname),
                    core::mem::transmute(&psztemplatename),
                    core::mem::transmute_copy(&dwfileattributes),
                    core::mem::transmute_copy(&hrnew),
                    core::mem::transmute_copy(&psinewitem),
                )
                .into()
            }
        }
        unsafe extern "system" fn UpdateProgress<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            iworktotal: u32,
            iworksofar: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::UpdateProgress(
                    this,
                    core::mem::transmute_copy(&iworktotal),
                    core::mem::transmute_copy(&iworksofar),
                )
                .into()
            }
        }
        unsafe extern "system" fn ResetTimer<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::ResetTimer(this).into()
            }
        }
        unsafe extern "system" fn PauseTimer<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::PauseTimer(this).into()
            }
        }
        unsafe extern "system" fn ResumeTimer<
            Identity: IFileOperationProgressSink_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IFileOperationProgressSink_Impl::ResumeTimer(this).into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            StartOperations: StartOperations::<Identity, OFFSET>,
            FinishOperations: FinishOperations::<Identity, OFFSET>,
            PreRenameItem: PreRenameItem::<Identity, OFFSET>,
            PostRenameItem: PostRenameItem::<Identity, OFFSET>,
            PreMoveItem: PreMoveItem::<Identity, OFFSET>,
            PostMoveItem: PostMoveItem::<Identity, OFFSET>,
            PreCopyItem: PreCopyItem::<Identity, OFFSET>,
            PostCopyItem: PostCopyItem::<Identity, OFFSET>,
            PreDeleteItem: PreDeleteItem::<Identity, OFFSET>,
            PostDeleteItem: PostDeleteItem::<Identity, OFFSET>,
            PreNewItem: PreNewItem::<Identity, OFFSET>,
            PostNewItem: PostNewItem::<Identity, OFFSET>,
            UpdateProgress: UpdateProgress::<Identity, OFFSET>,
            ResetTimer: ResetTimer::<Identity, OFFSET>,
            PauseTimer: PauseTimer::<Identity, OFFSET>,
            ResumeTimer: ResumeTimer::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IFileOperationProgressSink as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IFileOperationProgressSink {}
windows_core::imp::define_interface!(
    IMFActivate,
    IMFActivate_Vtbl,
    0x7fee9e9a_4a89_47a6_899c_b6a53a70fb67
);
impl core::ops::Deref for IMFActivate {
    type Target = IMFAttributes;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(IMFActivate, windows_core::IUnknown, IMFAttributes);
#[repr(C)]
pub struct IMFActivate_Vtbl {
    pub base__: IMFAttributes_Vtbl,
    ActivateObject: usize,
    ShutdownObject: usize,
    DetachObject: usize,
}
impl windows_core::RuntimeName for IMFActivate {}
windows_core::imp::define_interface!(
    IMFAsyncCallback,
    IMFAsyncCallback_Vtbl,
    0xa27003cf_2354_4f2a_8d6a_ab7cff15437e
);
windows_core::imp::interface_hierarchy!(IMFAsyncCallback, windows_core::IUnknown);
impl IMFAsyncCallback {
    pub unsafe fn GetParameters(
        &self,
        pdwflags: *mut u32,
        pdwqueue: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetParameters)(
                windows_core::Interface::as_raw(self),
                pdwflags as _,
                pdwqueue as _,
            )
        }
    }
    pub unsafe fn Invoke<P0>(&self, pasyncresult: P0) -> windows_core::HRESULT
    where
        P0: windows_core::Param<IMFAsyncResult>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).Invoke)(
                windows_core::Interface::as_raw(self),
                pasyncresult.param().abi(),
            )
        }
    }
}
#[repr(C)]
pub struct IMFAsyncCallback_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetParameters: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    pub Invoke: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
pub trait IMFAsyncCallback_Impl: windows_core::IUnknownImpl {
    fn GetParameters(&self, pdwflags: *mut u32, pdwqueue: *mut u32) -> windows_core::Result<()>;
    fn Invoke(&self, pasyncresult: windows_core::Ref<IMFAsyncResult>) -> windows_core::Result<()>;
}
impl IMFAsyncCallback_Vtbl {
    pub const fn new<Identity: IMFAsyncCallback_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn GetParameters<
            Identity: IMFAsyncCallback_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pdwflags: *mut u32,
            pdwqueue: *mut u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMFAsyncCallback_Impl::GetParameters(
                    this,
                    core::mem::transmute_copy(&pdwflags),
                    core::mem::transmute_copy(&pdwqueue),
                )
                .into()
            }
        }
        unsafe extern "system" fn Invoke<Identity: IMFAsyncCallback_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            pasyncresult: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMFAsyncCallback_Impl::Invoke(this, core::mem::transmute_copy(&pasyncresult)).into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            GetParameters: GetParameters::<Identity, OFFSET>,
            Invoke: Invoke::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IMFAsyncCallback as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IMFAsyncCallback {}
windows_core::imp::define_interface!(
    IMFAsyncResult,
    IMFAsyncResult_Vtbl,
    0xac6b7889_0740_4d51_8619_905994a55cc6
);
windows_core::imp::interface_hierarchy!(IMFAsyncResult, windows_core::IUnknown);
impl IMFAsyncResult {
    pub unsafe fn GetStatus(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetStatus)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetStatus(&self, hrstatus: windows_core::HRESULT) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetStatus)(
                windows_core::Interface::as_raw(self),
                hrstatus,
            )
        }
    }
    pub unsafe fn GetObject(&self) -> windows_core::Result<windows_core::IUnknown> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetObject)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IMFAsyncResult_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetState: usize,
    pub GetStatus: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub SetStatus: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::HRESULT,
    ) -> windows_core::HRESULT,
    pub GetObject: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    GetStateNoAddRef: usize,
}
impl windows_core::RuntimeName for IMFAsyncResult {}
windows_core::imp::define_interface!(
    IMFAttributes,
    IMFAttributes_Vtbl,
    0x2cd2d921_c447_44a7_a13c_4adabfc247e3
);
windows_core::imp::interface_hierarchy!(IMFAttributes, windows_core::IUnknown);
impl IMFAttributes {
    pub unsafe fn DeleteItem(&self, guidkey: *const windows_core::GUID) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).DeleteItem)(
                windows_core::Interface::as_raw(self),
                guidkey,
            )
        }
    }
    pub unsafe fn SetUINT32(
        &self,
        guidkey: *const windows_core::GUID,
        unvalue: u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetUINT32)(
                windows_core::Interface::as_raw(self),
                guidkey,
                unvalue,
            )
        }
    }
    pub unsafe fn SetString<P1>(
        &self,
        guidkey: *const windows_core::GUID,
        wszvalue: P1,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).SetString)(
                windows_core::Interface::as_raw(self),
                guidkey,
                wszvalue.param().abi(),
            )
        }
    }
    pub unsafe fn SetUnknown<P1>(
        &self,
        guidkey: *const windows_core::GUID,
        punknown: P1,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<windows_core::IUnknown>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).SetUnknown)(
                windows_core::Interface::as_raw(self),
                guidkey,
                punknown.param().abi(),
            )
        }
    }
}
#[repr(C)]
pub struct IMFAttributes_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetItem: usize,
    GetItemType: usize,
    CompareItem: usize,
    Compare: usize,
    GetUINT32: usize,
    GetUINT64: usize,
    GetDouble: usize,
    GetGUID: usize,
    GetStringLength: usize,
    GetString: usize,
    GetAllocatedString: usize,
    GetBlobSize: usize,
    GetBlob: usize,
    GetAllocatedBlob: usize,
    GetUnknown: usize,
    SetItem: usize,
    pub DeleteItem: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
    ) -> windows_core::HRESULT,
    DeleteAllItems: usize,
    pub SetUINT32: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        u32,
    ) -> windows_core::HRESULT,
    SetUINT64: usize,
    SetDouble: usize,
    SetGUID: usize,
    pub SetString: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    SetBlob: usize,
    pub SetUnknown: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    LockStore: usize,
    UnlockStore: usize,
    GetCount: usize,
    GetItemByIndex: usize,
    CopyAllItems: usize,
}
impl windows_core::RuntimeName for IMFAttributes {}
windows_core::imp::define_interface!(
    IMFByteStream,
    IMFByteStream_Vtbl,
    0xad4c1b00_4bf7_422f_9175_756693d9130d
);
windows_core::imp::interface_hierarchy!(IMFByteStream, windows_core::IUnknown);
impl IMFByteStream {
    pub unsafe fn GetLength(&self) -> windows_core::Result<u64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetLength)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn Read(&self, pb: *mut u8, cb: u32, pcbread: *mut u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Read)(
                windows_core::Interface::as_raw(self),
                pb as _,
                cb,
                pcbread as _,
            )
        }
    }
    pub unsafe fn Write(&self, pb: &[u8]) -> windows_core::Result<u32> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Write)(
                windows_core::Interface::as_raw(self),
                pb.as_ptr(),
                pb.len().try_into().unwrap(),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn Seek(
        &self,
        seekorigin: MFBYTESTREAM_SEEK_ORIGIN,
        llseekoffset: i64,
        dwseekflags: u32,
    ) -> windows_core::Result<u64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Seek)(
                windows_core::Interface::as_raw(self),
                seekorigin,
                llseekoffset,
                dwseekflags,
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn Flush(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Flush)(windows_core::Interface::as_raw(self))
        }
    }
}
#[repr(C)]
pub struct IMFByteStream_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetCapabilities: usize,
    pub GetLength:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u64) -> windows_core::HRESULT,
    SetLength: usize,
    GetCurrentPosition: usize,
    SetCurrentPosition: usize,
    IsEndOfStream: usize,
    pub Read: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut u8,
        u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    BeginRead: usize,
    EndRead: usize,
    pub Write: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const u8,
        u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    BeginWrite: usize,
    EndWrite: usize,
    pub Seek: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        MFBYTESTREAM_SEEK_ORIGIN,
        i64,
        u32,
        *mut u64,
    ) -> windows_core::HRESULT,
    pub Flush: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    Close: usize,
}
impl windows_core::RuntimeName for IMFByteStream {}
windows_core::imp::define_interface!(
    IMFDXGIDeviceManager,
    IMFDXGIDeviceManager_Vtbl,
    0xeb533d5d_2db6_40f8_97a9_494692014f07
);
windows_core::imp::interface_hierarchy!(IMFDXGIDeviceManager, windows_core::IUnknown);
impl IMFDXGIDeviceManager {
    pub unsafe fn ResetDevice<P0>(&self, punkdevice: P0, resettoken: u32) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::IUnknown>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).ResetDevice)(
                windows_core::Interface::as_raw(self),
                punkdevice.param().abi(),
                resettoken,
            )
        }
    }
}
#[repr(C)]
pub struct IMFDXGIDeviceManager_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    CloseDeviceHandle: usize,
    GetVideoService: usize,
    LockDevice: usize,
    OpenDeviceHandle: usize,
    pub ResetDevice: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        u32,
    ) -> windows_core::HRESULT,
    TestDevice: usize,
    UnlockDevice: usize,
}
impl windows_core::RuntimeName for IMFDXGIDeviceManager {}
windows_core::imp::define_interface!(
    IMFMediaEngine,
    IMFMediaEngine_Vtbl,
    0x98a1b0bb_03eb_4935_ae7c_93c1fa0e1c93
);
windows_core::imp::interface_hierarchy!(IMFMediaEngine, windows_core::IUnknown);
impl IMFMediaEngine {
    pub unsafe fn GetError(&self) -> windows_core::Result<IMFMediaError> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetError)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn SetSource(&self, purl: &windows_core::BSTR) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetSource)(
                windows_core::Interface::as_raw(self),
                core::mem::transmute_copy(purl),
            )
        }
    }
    pub unsafe fn GetNetworkState(&self) -> u16 {
        unsafe {
            (windows_core::Interface::vtable(self).GetNetworkState)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
    pub unsafe fn SetPreload(&self, preload: MF_MEDIA_ENGINE_PRELOAD) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetPreload)(
                windows_core::Interface::as_raw(self),
                preload,
            )
        }
    }
    pub unsafe fn GetBuffered(&self) -> windows_core::Result<IMFMediaTimeRange> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetBuffered)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn Load(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Load)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn GetReadyState(&self) -> u16 {
        unsafe {
            (windows_core::Interface::vtable(self).GetReadyState)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn IsSeeking(&self) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).IsSeeking)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn GetCurrentTime(&self) -> f64 {
        unsafe {
            (windows_core::Interface::vtable(self).GetCurrentTime)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn SetCurrentTime(&self, seektime: f64) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetCurrentTime)(
                windows_core::Interface::as_raw(self),
                seektime,
            )
        }
    }
    pub unsafe fn GetDuration(&self) -> f64 {
        unsafe {
            (windows_core::Interface::vtable(self).GetDuration)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn IsPaused(&self) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).IsPaused)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetPlaybackRate(&self, rate: f64) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetPlaybackRate)(
                windows_core::Interface::as_raw(self),
                rate,
            )
        }
    }
    pub unsafe fn GetSeekable(&self) -> windows_core::Result<IMFMediaTimeRange> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetSeekable)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn IsEnded(&self) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).IsEnded)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetLoop(&self, r#loop: bool) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetLoop)(
                windows_core::Interface::as_raw(self),
                r#loop.into(),
            )
        }
    }
    pub unsafe fn Play(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Play)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn Pause(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Pause)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn GetMuted(&self) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).GetMuted)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetMuted(&self, muted: bool) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetMuted)(
                windows_core::Interface::as_raw(self),
                muted.into(),
            )
        }
    }
    pub unsafe fn GetVolume(&self) -> f64 {
        unsafe {
            (windows_core::Interface::vtable(self).GetVolume)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn SetVolume(&self, volume: f64) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetVolume)(
                windows_core::Interface::as_raw(self),
                volume,
            )
        }
    }
    pub unsafe fn HasAudio(&self) -> windows_core::BOOL {
        unsafe {
            (windows_core::Interface::vtable(self).HasAudio)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn GetNativeVideoSize(
        &self,
        cx: Option<*mut u32>,
        cy: Option<*mut u32>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetNativeVideoSize)(
                windows_core::Interface::as_raw(self),
                cx.unwrap_or(core::mem::zeroed()) as _,
                cy.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn Shutdown(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Shutdown)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn TransferVideoFrame<P0>(
        &self,
        pdstsurf: P0,
        psrc: Option<*const MFVideoNormalizedRect>,
        pdst: *const RECT,
        pborderclr: Option<*const MFARGB>,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::IUnknown>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).TransferVideoFrame)(
                windows_core::Interface::as_raw(self),
                pdstsurf.param().abi(),
                psrc.unwrap_or(core::mem::zeroed()) as _,
                pdst,
                pborderclr.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn OnVideoStreamTick(&self) -> windows_core::Result<i64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).OnVideoStreamTick)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngine_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetError: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    SetErrorCode: usize,
    SetSourceElements: usize,
    pub SetSource: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    GetCurrentSource: usize,
    pub GetNetworkState: unsafe extern "system" fn(*mut core::ffi::c_void) -> u16,
    GetPreload: usize,
    pub SetPreload: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        MF_MEDIA_ENGINE_PRELOAD,
    ) -> windows_core::HRESULT,
    pub GetBuffered: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub Load: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    CanPlayType: usize,
    pub GetReadyState: unsafe extern "system" fn(*mut core::ffi::c_void) -> u16,
    pub IsSeeking: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::BOOL,
    pub GetCurrentTime: unsafe extern "system" fn(*mut core::ffi::c_void) -> f64,
    pub SetCurrentTime:
        unsafe extern "system" fn(*mut core::ffi::c_void, f64) -> windows_core::HRESULT,
    GetStartTime: usize,
    pub GetDuration: unsafe extern "system" fn(*mut core::ffi::c_void) -> f64,
    pub IsPaused: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::BOOL,
    GetDefaultPlaybackRate: usize,
    SetDefaultPlaybackRate: usize,
    GetPlaybackRate: usize,
    pub SetPlaybackRate:
        unsafe extern "system" fn(*mut core::ffi::c_void, f64) -> windows_core::HRESULT,
    GetPlayed: usize,
    pub GetSeekable: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub IsEnded: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::BOOL,
    GetAutoPlay: usize,
    SetAutoPlay: usize,
    GetLoop: usize,
    pub SetLoop: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::BOOL,
    ) -> windows_core::HRESULT,
    pub Play: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub Pause: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub GetMuted: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::BOOL,
    pub SetMuted: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::BOOL,
    ) -> windows_core::HRESULT,
    pub GetVolume: unsafe extern "system" fn(*mut core::ffi::c_void) -> f64,
    pub SetVolume: unsafe extern "system" fn(*mut core::ffi::c_void, f64) -> windows_core::HRESULT,
    HasVideo: usize,
    pub HasAudio: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::BOOL,
    pub GetNativeVideoSize: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    GetVideoAspectRatio: usize,
    pub Shutdown: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub TransferVideoFrame: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *const MFVideoNormalizedRect,
        *const RECT,
        *const MFARGB,
    ) -> windows_core::HRESULT,
    pub OnVideoStreamTick:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut i64) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IMFMediaEngine {}
windows_core::imp::define_interface!(
    IMFMediaEngineClassFactory,
    IMFMediaEngineClassFactory_Vtbl,
    0x4d645ace_26aa_4688_9be1_df3516990b93
);
windows_core::imp::interface_hierarchy!(IMFMediaEngineClassFactory, windows_core::IUnknown);
impl IMFMediaEngineClassFactory {
    pub unsafe fn CreateInstance<P1>(
        &self,
        dwflags: u32,
        pattr: P1,
    ) -> windows_core::Result<IMFMediaEngine>
    where
        P1: windows_core::Param<IMFAttributes>,
    {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).CreateInstance)(
                windows_core::Interface::as_raw(self),
                dwflags,
                pattr.param().abi(),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngineClassFactory_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub CreateInstance: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    CreateTimeRange: usize,
    CreateError: usize,
}
impl windows_core::RuntimeName for IMFMediaEngineClassFactory {}
windows_core::imp::define_interface!(
    IMFMediaEngineEx,
    IMFMediaEngineEx_Vtbl,
    0x83015ead_b1e6_40d0_a98a_37145ffe1ad1
);
impl core::ops::Deref for IMFMediaEngineEx {
    type Target = IMFMediaEngine;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(IMFMediaEngineEx, windows_core::IUnknown, IMFMediaEngine);
impl IMFMediaEngineEx {
    pub unsafe fn SetSourceFromByteStream<P0>(
        &self,
        pbytestream: P0,
        purl: &windows_core::BSTR,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<IMFByteStream>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).SetSourceFromByteStream)(
                windows_core::Interface::as_raw(self),
                pbytestream.param().abi(),
                core::mem::transmute_copy(purl),
            )
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngineEx_Vtbl {
    pub base__: IMFMediaEngine_Vtbl,
    pub SetSourceFromByteStream: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    GetStatistics: usize,
    UpdateVideoStream: usize,
    GetBalance: usize,
    SetBalance: usize,
    IsPlaybackRateSupported: usize,
    FrameStep: usize,
    GetResourceCharacteristics: usize,
    GetPresentationAttribute: usize,
    GetNumberOfStreams: usize,
    GetStreamAttribute: usize,
    GetStreamSelection: usize,
    SetStreamSelection: usize,
    ApplyStreamSelections: usize,
    IsProtected: usize,
    InsertVideoEffect: usize,
    InsertAudioEffect: usize,
    RemoveAllEffects: usize,
    SetTimelineMarkerTimer: usize,
    GetTimelineMarkerTimer: usize,
    CancelTimelineMarkerTimer: usize,
    IsStereo3D: usize,
    GetStereo3DFramePackingMode: usize,
    SetStereo3DFramePackingMode: usize,
    GetStereo3DRenderMode: usize,
    SetStereo3DRenderMode: usize,
    EnableWindowlessSwapchainMode: usize,
    GetVideoSwapchainHandle: usize,
    EnableHorizontalMirrorMode: usize,
    GetAudioStreamCategory: usize,
    SetAudioStreamCategory: usize,
    GetAudioEndpointRole: usize,
    SetAudioEndpointRole: usize,
    GetRealTimeMode: usize,
    SetRealTimeMode: usize,
    SetCurrentTimeEx: usize,
    EnableTimeUpdateTimer: usize,
}
impl windows_core::RuntimeName for IMFMediaEngineEx {}
windows_core::imp::define_interface!(
    IMFMediaEngineExtension,
    IMFMediaEngineExtension_Vtbl,
    0x2f69d622_20b5_41e9_afdf_89ced1dda04e
);
windows_core::imp::interface_hierarchy!(IMFMediaEngineExtension, windows_core::IUnknown);
impl IMFMediaEngineExtension {
    pub unsafe fn CanPlayType(
        &self,
        audioonly: bool,
        mimetype: &windows_core::BSTR,
    ) -> windows_core::Result<MF_MEDIA_ENGINE_CANPLAY> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).CanPlayType)(
                windows_core::Interface::as_raw(self),
                audioonly.into(),
                core::mem::transmute_copy(mimetype),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn BeginCreateObject<P1, P4, P5>(
        &self,
        bstrurl: &windows_core::BSTR,
        pbytestream: P1,
        r#type: MF_OBJECT_TYPE,
        ppiunknowncancelcookie: *mut Option<windows_core::IUnknown>,
        pcallback: P4,
        punkstate: P5,
    ) -> windows_core::HRESULT
    where
        P1: windows_core::Param<IMFByteStream>,
        P4: windows_core::Param<IMFAsyncCallback>,
        P5: windows_core::Param<windows_core::IUnknown>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).BeginCreateObject)(
                windows_core::Interface::as_raw(self),
                core::mem::transmute_copy(bstrurl),
                pbytestream.param().abi(),
                r#type,
                core::mem::transmute(ppiunknowncancelcookie),
                pcallback.param().abi(),
                punkstate.param().abi(),
            )
        }
    }
    pub unsafe fn CancelObjectCreation<P0>(
        &self,
        piunknowncancelcookie: P0,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::IUnknown>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).CancelObjectCreation)(
                windows_core::Interface::as_raw(self),
                piunknowncancelcookie.param().abi(),
            )
        }
    }
    pub unsafe fn EndCreateObject<P0>(
        &self,
        presult: P0,
    ) -> windows_core::Result<windows_core::IUnknown>
    where
        P0: windows_core::Param<IMFAsyncResult>,
    {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).EndCreateObject)(
                windows_core::Interface::as_raw(self),
                presult.param().abi(),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngineExtension_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub CanPlayType: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::BOOL,
        *mut core::ffi::c_void,
        *mut MF_MEDIA_ENGINE_CANPLAY,
    ) -> windows_core::HRESULT,
    pub BeginCreateObject: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        MF_OBJECT_TYPE,
        *mut *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub CancelObjectCreation: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub EndCreateObject: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
pub trait IMFMediaEngineExtension_Impl: windows_core::IUnknownImpl {
    fn CanPlayType(
        &self,
        audioonly: windows_core::BOOL,
        mimetype: &windows_core::BSTR,
    ) -> windows_core::Result<MF_MEDIA_ENGINE_CANPLAY>;
    fn BeginCreateObject(
        &self,
        bstrurl: &windows_core::BSTR,
        pbytestream: windows_core::Ref<IMFByteStream>,
        r#type: MF_OBJECT_TYPE,
        ppiunknowncancelcookie: windows_core::OutRef<windows_core::IUnknown>,
        pcallback: windows_core::Ref<IMFAsyncCallback>,
        punkstate: windows_core::Ref<windows_core::IUnknown>,
    ) -> windows_core::Result<()>;
    fn CancelObjectCreation(
        &self,
        piunknowncancelcookie: windows_core::Ref<windows_core::IUnknown>,
    ) -> windows_core::Result<()>;
    fn EndCreateObject(
        &self,
        presult: windows_core::Ref<IMFAsyncResult>,
    ) -> windows_core::Result<windows_core::IUnknown>;
}
impl IMFMediaEngineExtension_Vtbl {
    pub const fn new<Identity: IMFMediaEngineExtension_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn CanPlayType<
            Identity: IMFMediaEngineExtension_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            audioonly: windows_core::BOOL,
            mimetype: *mut core::ffi::c_void,
            panswer: *mut MF_MEDIA_ENGINE_CANPLAY,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                match IMFMediaEngineExtension_Impl::CanPlayType(
                    this,
                    core::mem::transmute_copy(&audioonly),
                    core::mem::transmute(&mimetype),
                ) {
                    Ok(ok__) => {
                        panswer.write(ok__);
                        windows_core::HRESULT(0)
                    }
                    Err(err) => err.into(),
                }
            }
        }
        unsafe extern "system" fn BeginCreateObject<
            Identity: IMFMediaEngineExtension_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            bstrurl: *mut core::ffi::c_void,
            pbytestream: *mut core::ffi::c_void,
            r#type: MF_OBJECT_TYPE,
            ppiunknowncancelcookie: *mut *mut core::ffi::c_void,
            pcallback: *mut core::ffi::c_void,
            punkstate: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMFMediaEngineExtension_Impl::BeginCreateObject(
                    this,
                    core::mem::transmute(&bstrurl),
                    core::mem::transmute_copy(&pbytestream),
                    core::mem::transmute_copy(&r#type),
                    core::mem::transmute_copy(&ppiunknowncancelcookie),
                    core::mem::transmute_copy(&pcallback),
                    core::mem::transmute_copy(&punkstate),
                )
                .into()
            }
        }
        unsafe extern "system" fn CancelObjectCreation<
            Identity: IMFMediaEngineExtension_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            piunknowncancelcookie: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMFMediaEngineExtension_Impl::CancelObjectCreation(
                    this,
                    core::mem::transmute_copy(&piunknowncancelcookie),
                )
                .into()
            }
        }
        unsafe extern "system" fn EndCreateObject<
            Identity: IMFMediaEngineExtension_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            presult: *mut core::ffi::c_void,
            ppobject: *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                match IMFMediaEngineExtension_Impl::EndCreateObject(
                    this,
                    core::mem::transmute_copy(&presult),
                ) {
                    Ok(ok__) => {
                        ppobject.write(core::mem::transmute(ok__));
                        windows_core::HRESULT(0)
                    }
                    Err(err) => err.into(),
                }
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            CanPlayType: CanPlayType::<Identity, OFFSET>,
            BeginCreateObject: BeginCreateObject::<Identity, OFFSET>,
            CancelObjectCreation: CancelObjectCreation::<Identity, OFFSET>,
            EndCreateObject: EndCreateObject::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IMFMediaEngineExtension as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IMFMediaEngineExtension {}
windows_core::imp::define_interface!(
    IMFMediaEngineNotify,
    IMFMediaEngineNotify_Vtbl,
    0xfee7c112_e776_42b5_9bbf_0048524e2bd5
);
windows_core::imp::interface_hierarchy!(IMFMediaEngineNotify, windows_core::IUnknown);
impl IMFMediaEngineNotify {
    pub unsafe fn EventNotify(
        &self,
        event: u32,
        param1: usize,
        param2: u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).EventNotify)(
                windows_core::Interface::as_raw(self),
                event,
                param1,
                param2,
            )
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngineNotify_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub EventNotify:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32, usize, u32) -> windows_core::HRESULT,
}
pub trait IMFMediaEngineNotify_Impl: windows_core::IUnknownImpl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows_core::Result<()>;
}
impl IMFMediaEngineNotify_Vtbl {
    pub const fn new<Identity: IMFMediaEngineNotify_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn EventNotify<
            Identity: IMFMediaEngineNotify_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            event: u32,
            param1: usize,
            param2: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMFMediaEngineNotify_Impl::EventNotify(
                    this,
                    core::mem::transmute_copy(&event),
                    core::mem::transmute_copy(&param1),
                    core::mem::transmute_copy(&param2),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            EventNotify: EventNotify::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IMFMediaEngineNotify as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IMFMediaEngineNotify {}
windows_core::imp::define_interface!(
    IMFMediaEngineWebSupport,
    IMFMediaEngineWebSupport_Vtbl,
    0xba2743a1_07e0_48ef_84b6_9a2ed023ca6c
);
windows_core::imp::interface_hierarchy!(IMFMediaEngineWebSupport, windows_core::IUnknown);
impl IMFMediaEngineWebSupport {
    pub unsafe fn ConnectWebAudio(
        &self,
        dwsamplerate: u32,
    ) -> windows_core::Result<IAudioSourceProvider> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ConnectWebAudio)(
                windows_core::Interface::as_raw(self),
                dwsamplerate,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn DisconnectWebAudio(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).DisconnectWebAudio)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
}
#[repr(C)]
pub struct IMFMediaEngineWebSupport_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    ShouldDelayTheLoadEvent: usize,
    pub ConnectWebAudio: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        u32,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DisconnectWebAudio:
        unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IMFMediaEngineWebSupport {}
windows_core::imp::define_interface!(
    IMFMediaError,
    IMFMediaError_Vtbl,
    0xfc0e10d2_ab2a_4501_a951_06bb1075184c
);
windows_core::imp::interface_hierarchy!(IMFMediaError, windows_core::IUnknown);
impl IMFMediaError {
    pub unsafe fn GetErrorCode(&self) -> u16 {
        unsafe {
            (windows_core::Interface::vtable(self).GetErrorCode)(windows_core::Interface::as_raw(
                self,
            ))
        }
    }
    pub unsafe fn GetExtendedErrorCode(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetExtendedErrorCode)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
}
#[repr(C)]
pub struct IMFMediaError_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetErrorCode: unsafe extern "system" fn(*mut core::ffi::c_void) -> u16,
    pub GetExtendedErrorCode:
        unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    SetErrorCode: usize,
    SetExtendedErrorCode: usize,
}
impl windows_core::RuntimeName for IMFMediaError {}
windows_core::imp::define_interface!(
    IMFMediaSink,
    IMFMediaSink_Vtbl,
    0x6ef2a660_47c0_4666_b13d_cbb717f2fa2c
);
windows_core::imp::interface_hierarchy!(IMFMediaSink, windows_core::IUnknown);
impl IMFMediaSink {
    pub unsafe fn Shutdown(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Shutdown)(windows_core::Interface::as_raw(self))
        }
    }
}
#[repr(C)]
pub struct IMFMediaSink_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetCharacteristics: usize,
    AddStreamSink: usize,
    RemoveStreamSink: usize,
    GetStreamSinkCount: usize,
    GetStreamSinkByIndex: usize,
    GetStreamSinkById: usize,
    SetPresentationClock: usize,
    GetPresentationClock: usize,
    pub Shutdown: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for IMFMediaSink {}
windows_core::imp::define_interface!(
    IMFMediaTimeRange,
    IMFMediaTimeRange_Vtbl,
    0xdb71a2fc_078a_414e_9df9_8c2531b0aa6c
);
windows_core::imp::interface_hierarchy!(IMFMediaTimeRange, windows_core::IUnknown);
impl IMFMediaTimeRange {
    pub unsafe fn GetLength(&self) -> u32 {
        unsafe {
            (windows_core::Interface::vtable(self).GetLength)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn GetStart(&self, index: u32) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetStart)(
                windows_core::Interface::as_raw(self),
                index,
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn GetEnd(&self, index: u32) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetEnd)(
                windows_core::Interface::as_raw(self),
                index,
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IMFMediaTimeRange_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetLength: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    pub GetStart:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32, *mut f64) -> windows_core::HRESULT,
    pub GetEnd:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32, *mut f64) -> windows_core::HRESULT,
    ContainsTime: usize,
    AddRange: usize,
    Clear: usize,
}
impl windows_core::RuntimeName for IMFMediaTimeRange {}
windows_core::imp::define_interface!(
    IMMDevice,
    IMMDevice_Vtbl,
    0xd666063f_1587_4e43_81f1_b948e807363f
);
windows_core::imp::interface_hierarchy!(IMMDevice, windows_core::IUnknown);
impl IMMDevice {
    pub unsafe fn Activate<T>(
        &self,
        dwclsctx: CLSCTX,
        pactivationparams: Option<*const PROPVARIANT>,
    ) -> windows_core::Result<T>
    where
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).Activate)(
                windows_core::Interface::as_raw(self),
                &T::IID,
                dwclsctx,
                pactivationparams.unwrap_or(core::mem::zeroed()) as _,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn GetId(&self) -> windows_core::Result<windows_core::PWSTR> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetId)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IMMDevice_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub Activate: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        CLSCTX,
        *const PROPVARIANT,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    OpenPropertyStore: usize,
    pub GetId: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut windows_core::PWSTR,
    ) -> windows_core::HRESULT,
    GetState: usize,
}
impl windows_core::RuntimeName for IMMDevice {}
windows_core::imp::define_interface!(
    IMMDeviceEnumerator,
    IMMDeviceEnumerator_Vtbl,
    0xa95664d2_9614_4f35_a746_de8db63617e6
);
windows_core::imp::interface_hierarchy!(IMMDeviceEnumerator, windows_core::IUnknown);
impl IMMDeviceEnumerator {
    pub unsafe fn GetDefaultAudioEndpoint(
        &self,
        dataflow: EDataFlow,
        role: ERole,
    ) -> windows_core::Result<IMMDevice> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetDefaultAudioEndpoint)(
                windows_core::Interface::as_raw(self),
                dataflow,
                role,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn GetDevice<P0>(&self, pwstrid: P0) -> windows_core::Result<IMMDevice>
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetDevice)(
                windows_core::Interface::as_raw(self),
                pwstrid.param().abi(),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn RegisterEndpointNotificationCallback<P0>(
        &self,
        pclient: P0,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<IMMNotificationClient>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).RegisterEndpointNotificationCallback)(
                windows_core::Interface::as_raw(self),
                pclient.param().abi(),
            )
        }
    }
}
#[repr(C)]
pub struct IMMDeviceEnumerator_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    EnumAudioEndpoints: usize,
    pub GetDefaultAudioEndpoint: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        EDataFlow,
        ERole,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub GetDevice: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub RegisterEndpointNotificationCallback: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    )
        -> windows_core::HRESULT,
    UnregisterEndpointNotificationCallback: usize,
}
impl windows_core::RuntimeName for IMMDeviceEnumerator {}
windows_core::imp::define_interface!(
    IMMNotificationClient,
    IMMNotificationClient_Vtbl,
    0x7991eec9_7e89_4d85_8390_6c703cec60c0
);
windows_core::imp::interface_hierarchy!(IMMNotificationClient, windows_core::IUnknown);
impl IMMNotificationClient {
    pub unsafe fn OnDeviceStateChanged<P0>(
        &self,
        pwstrdeviceid: P0,
        dwnewstate: DEVICE_STATE,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).OnDeviceStateChanged)(
                windows_core::Interface::as_raw(self),
                pwstrdeviceid.param().abi(),
                dwnewstate,
            )
        }
    }
    pub unsafe fn OnDeviceAdded<P0>(&self, pwstrdeviceid: P0) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).OnDeviceAdded)(
                windows_core::Interface::as_raw(self),
                pwstrdeviceid.param().abi(),
            )
        }
    }
    pub unsafe fn OnDeviceRemoved<P0>(&self, pwstrdeviceid: P0) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).OnDeviceRemoved)(
                windows_core::Interface::as_raw(self),
                pwstrdeviceid.param().abi(),
            )
        }
    }
    pub unsafe fn OnDefaultDeviceChanged<P2>(
        &self,
        flow: EDataFlow,
        role: ERole,
        pwstrdefaultdeviceid: P2,
    ) -> windows_core::HRESULT
    where
        P2: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).OnDefaultDeviceChanged)(
                windows_core::Interface::as_raw(self),
                flow,
                role,
                pwstrdefaultdeviceid.param().abi(),
            )
        }
    }
    pub unsafe fn OnPropertyValueChanged<P0>(
        &self,
        pwstrdeviceid: P0,
        key: PROPERTYKEY,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).OnPropertyValueChanged)(
                windows_core::Interface::as_raw(self),
                pwstrdeviceid.param().abi(),
                key,
            )
        }
    }
}
#[repr(C)]
pub struct IMMNotificationClient_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub OnDeviceStateChanged: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        DEVICE_STATE,
    ) -> windows_core::HRESULT,
    pub OnDeviceAdded: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub OnDeviceRemoved: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub OnDefaultDeviceChanged: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        EDataFlow,
        ERole,
        windows_core::PCWSTR,
    ) -> windows_core::HRESULT,
    pub OnPropertyValueChanged: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        PROPERTYKEY,
    ) -> windows_core::HRESULT,
}
pub trait IMMNotificationClient_Impl: windows_core::IUnknownImpl {
    fn OnDeviceStateChanged(
        &self,
        pwstrdeviceid: &windows_core::PCWSTR,
        dwnewstate: DEVICE_STATE,
    ) -> windows_core::Result<()>;
    fn OnDeviceAdded(&self, pwstrdeviceid: &windows_core::PCWSTR) -> windows_core::Result<()>;
    fn OnDeviceRemoved(&self, pwstrdeviceid: &windows_core::PCWSTR) -> windows_core::Result<()>;
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        pwstrdefaultdeviceid: &windows_core::PCWSTR,
    ) -> windows_core::Result<()>;
    fn OnPropertyValueChanged(
        &self,
        pwstrdeviceid: &windows_core::PCWSTR,
        key: &PROPERTYKEY,
    ) -> windows_core::Result<()>;
}
impl IMMNotificationClient_Vtbl {
    pub const fn new<Identity: IMMNotificationClient_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn OnDeviceStateChanged<
            Identity: IMMNotificationClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pwstrdeviceid: windows_core::PCWSTR,
            dwnewstate: DEVICE_STATE,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMMNotificationClient_Impl::OnDeviceStateChanged(
                    this,
                    core::mem::transmute(&pwstrdeviceid),
                    core::mem::transmute_copy(&dwnewstate),
                )
                .into()
            }
        }
        unsafe extern "system" fn OnDeviceAdded<
            Identity: IMMNotificationClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pwstrdeviceid: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMMNotificationClient_Impl::OnDeviceAdded(
                    this,
                    core::mem::transmute(&pwstrdeviceid),
                )
                .into()
            }
        }
        unsafe extern "system" fn OnDeviceRemoved<
            Identity: IMMNotificationClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pwstrdeviceid: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMMNotificationClient_Impl::OnDeviceRemoved(
                    this,
                    core::mem::transmute(&pwstrdeviceid),
                )
                .into()
            }
        }
        unsafe extern "system" fn OnDefaultDeviceChanged<
            Identity: IMMNotificationClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            flow: EDataFlow,
            role: ERole,
            pwstrdefaultdeviceid: windows_core::PCWSTR,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMMNotificationClient_Impl::OnDefaultDeviceChanged(
                    this,
                    core::mem::transmute_copy(&flow),
                    core::mem::transmute_copy(&role),
                    core::mem::transmute(&pwstrdefaultdeviceid),
                )
                .into()
            }
        }
        unsafe extern "system" fn OnPropertyValueChanged<
            Identity: IMMNotificationClient_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            pwstrdeviceid: windows_core::PCWSTR,
            key: PROPERTYKEY,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IMMNotificationClient_Impl::OnPropertyValueChanged(
                    this,
                    core::mem::transmute(&pwstrdeviceid),
                    core::mem::transmute(&key),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            OnDeviceStateChanged: OnDeviceStateChanged::<Identity, OFFSET>,
            OnDeviceAdded: OnDeviceAdded::<Identity, OFFSET>,
            OnDeviceRemoved: OnDeviceRemoved::<Identity, OFFSET>,
            OnDefaultDeviceChanged: OnDefaultDeviceChanged::<Identity, OFFSET>,
            OnPropertyValueChanged: OnPropertyValueChanged::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IMMNotificationClient as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IMMNotificationClient {}
windows_core::imp::define_interface!(
    INotificationActivationCallback,
    INotificationActivationCallback_Vtbl,
    0x53e31837_6600_4a81_9395_75cffe746f94
);
windows_core::imp::interface_hierarchy!(INotificationActivationCallback, windows_core::IUnknown);
impl INotificationActivationCallback {
    pub unsafe fn Activate<P0, P1>(
        &self,
        appusermodelid: P0,
        invokedargs: P1,
        data: &[NOTIFICATION_USER_INPUT_DATA],
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<windows_core::PCWSTR>,
        P1: windows_core::Param<windows_core::PCWSTR>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).Activate)(
                windows_core::Interface::as_raw(self),
                appusermodelid.param().abi(),
                invokedargs.param().abi(),
                data.as_ptr(),
                data.len().try_into().unwrap(),
            )
        }
    }
}
#[repr(C)]
pub struct INotificationActivationCallback_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub Activate: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        windows_core::PCWSTR,
        windows_core::PCWSTR,
        *const NOTIFICATION_USER_INPUT_DATA,
        u32,
    ) -> windows_core::HRESULT,
}
pub trait INotificationActivationCallback_Impl: windows_core::IUnknownImpl {
    fn Activate(
        &self,
        appusermodelid: &windows_core::PCWSTR,
        invokedargs: &windows_core::PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> windows_core::Result<()>;
}
impl INotificationActivationCallback_Vtbl {
    pub const fn new<Identity: INotificationActivationCallback_Impl, const OFFSET: isize>() -> Self
    {
        unsafe extern "system" fn Activate<
            Identity: INotificationActivationCallback_Impl,
            const OFFSET: isize,
        >(
            this: *mut core::ffi::c_void,
            appusermodelid: windows_core::PCWSTR,
            invokedargs: windows_core::PCWSTR,
            data: *const NOTIFICATION_USER_INPUT_DATA,
            count: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                INotificationActivationCallback_Impl::Activate(
                    this,
                    core::mem::transmute(&appusermodelid),
                    core::mem::transmute(&invokedargs),
                    core::mem::transmute_copy(&data),
                    core::mem::transmute_copy(&count),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            Activate: Activate::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<INotificationActivationCallback as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for INotificationActivationCallback {}
windows_core::imp::define_interface!(
    ISequentialStream,
    ISequentialStream_Vtbl,
    0x0c733a30_2a1c_11ce_ade5_00aa0044773d
);
windows_core::imp::interface_hierarchy!(ISequentialStream, windows_core::IUnknown);
impl ISequentialStream {
    pub unsafe fn Read(
        &self,
        pv: *mut core::ffi::c_void,
        cb: u32,
        pcbread: Option<*mut u32>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Read)(
                windows_core::Interface::as_raw(self),
                pv as _,
                cb,
                pcbread.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn Write(
        &self,
        pv: *const core::ffi::c_void,
        cb: u32,
        pcbwritten: Option<*mut u32>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Write)(
                windows_core::Interface::as_raw(self),
                pv,
                cb,
                pcbwritten.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
}
#[repr(C)]
pub struct ISequentialStream_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub Read: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    pub Write: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const core::ffi::c_void,
        u32,
        *mut u32,
    ) -> windows_core::HRESULT,
}
pub trait ISequentialStream_Impl: windows_core::IUnknownImpl {
    fn Read(
        &self,
        pv: *mut core::ffi::c_void,
        cb: u32,
        pcbread: *mut u32,
    ) -> windows_core::Result<()>;
    fn Write(
        &self,
        pv: *const core::ffi::c_void,
        cb: u32,
        pcbwritten: *mut u32,
    ) -> windows_core::Result<()>;
}
impl ISequentialStream_Vtbl {
    pub const fn new<Identity: ISequentialStream_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn Read<Identity: ISequentialStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            pv: *mut core::ffi::c_void,
            cb: u32,
            pcbread: *mut u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                ISequentialStream_Impl::Read(
                    this,
                    core::mem::transmute_copy(&pv),
                    core::mem::transmute_copy(&cb),
                    core::mem::transmute_copy(&pcbread),
                )
                .into()
            }
        }
        unsafe extern "system" fn Write<Identity: ISequentialStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            pv: *const core::ffi::c_void,
            cb: u32,
            pcbwritten: *mut u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                ISequentialStream_Impl::Write(
                    this,
                    core::mem::transmute_copy(&pv),
                    core::mem::transmute_copy(&cb),
                    core::mem::transmute_copy(&pcbwritten),
                )
                .into()
            }
        }
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            Read: Read::<Identity, OFFSET>,
            Write: Write::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<ISequentialStream as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for ISequentialStream {}
windows_core::imp::define_interface!(
    IShellItem,
    IShellItem_Vtbl,
    0x43826d1e_e718_42ee_bc55_a1e261c37bfe
);
windows_core::imp::interface_hierarchy!(IShellItem, windows_core::IUnknown);
#[repr(C)]
pub struct IShellItem_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    BindToHandler: usize,
    GetParent: usize,
    GetDisplayName: usize,
    GetAttributes: usize,
    Compare: usize,
}
impl windows_core::RuntimeName for IShellItem {}
windows_core::imp::define_interface!(
    ISpatialAudioClient,
    ISpatialAudioClient_Vtbl,
    0xbbf8e066_aaaa_49be_9a4d_fd2a858ea27f
);
windows_core::imp::interface_hierarchy!(ISpatialAudioClient, windows_core::IUnknown);
impl ISpatialAudioClient {
    pub unsafe fn GetNativeStaticObjectTypeMask(&self) -> windows_core::Result<AudioObjectType> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetNativeStaticObjectTypeMask)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn GetMaxDynamicObjectCount(&self) -> windows_core::Result<u32> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetMaxDynamicObjectCount)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn GetSupportedAudioObjectFormatEnumerator(
        &self,
    ) -> windows_core::Result<IAudioFormatEnumerator> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetSupportedAudioObjectFormatEnumerator)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn GetMaxFrameCount(
        &self,
        objectformat: *const WAVEFORMATEX,
    ) -> windows_core::Result<u32> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).GetMaxFrameCount)(
                windows_core::Interface::as_raw(self),
                objectformat,
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub unsafe fn ActivateSpatialAudioStream<T>(
        &self,
        activationparams: *const PROPVARIANT,
    ) -> windows_core::Result<T>
    where
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).ActivateSpatialAudioStream)(
                windows_core::Interface::as_raw(self),
                activationparams,
                &T::IID,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct ISpatialAudioClient_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetStaticObjectPosition: usize,
    pub GetNativeStaticObjectTypeMask: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut AudioObjectType,
    ) -> windows_core::HRESULT,
    pub GetMaxDynamicObjectCount:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32) -> windows_core::HRESULT,
    pub GetSupportedAudioObjectFormatEnumerator: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    )
        -> windows_core::HRESULT,
    pub GetMaxFrameCount: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const WAVEFORMATEX,
        *mut u32,
    ) -> windows_core::HRESULT,
    IsAudioObjectFormatSupported: usize,
    IsSpatialAudioStreamAvailable: usize,
    pub ActivateSpatialAudioStream: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const PROPVARIANT,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for ISpatialAudioClient {}
windows_core::imp::define_interface!(
    ISpatialAudioObject,
    ISpatialAudioObject_Vtbl,
    0xdde28967_521b_46e5_8f00_bd6f2bc8ab1d
);
impl core::ops::Deref for ISpatialAudioObject {
    type Target = ISpatialAudioObjectBase;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(
    ISpatialAudioObject,
    windows_core::IUnknown,
    ISpatialAudioObjectBase
);
impl ISpatialAudioObject {
    pub unsafe fn SetPosition(&self, x: f32, y: f32, z: f32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetPosition)(
                windows_core::Interface::as_raw(self),
                x,
                y,
                z,
            )
        }
    }
    pub unsafe fn SetVolume(&self, volume: f32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetVolume)(
                windows_core::Interface::as_raw(self),
                volume,
            )
        }
    }
}
#[repr(C)]
pub struct ISpatialAudioObject_Vtbl {
    pub base__: ISpatialAudioObjectBase_Vtbl,
    pub SetPosition:
        unsafe extern "system" fn(*mut core::ffi::c_void, f32, f32, f32) -> windows_core::HRESULT,
    pub SetVolume: unsafe extern "system" fn(*mut core::ffi::c_void, f32) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for ISpatialAudioObject {}
windows_core::imp::define_interface!(
    ISpatialAudioObjectBase,
    ISpatialAudioObjectBase_Vtbl,
    0xcce0b8f2_8d4d_4efb_a8cf_3d6ecf1c30e0
);
windows_core::imp::interface_hierarchy!(ISpatialAudioObjectBase, windows_core::IUnknown);
impl ISpatialAudioObjectBase {
    pub unsafe fn GetBuffer(
        &self,
        buffer: *mut *mut u8,
        bufferlength: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).GetBuffer)(
                windows_core::Interface::as_raw(self),
                buffer as _,
                bufferlength as _,
            )
        }
    }
    pub unsafe fn SetEndOfStream(&self, framecount: u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetEndOfStream)(
                windows_core::Interface::as_raw(self),
                framecount,
            )
        }
    }
}
#[repr(C)]
pub struct ISpatialAudioObjectBase_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    pub GetBuffer: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut u8,
        *mut u32,
    ) -> windows_core::HRESULT,
    pub SetEndOfStream:
        unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> windows_core::HRESULT,
    IsActive: usize,
    GetAudioObjectType: usize,
}
impl windows_core::RuntimeName for ISpatialAudioObjectBase {}
windows_core::imp::define_interface!(
    ISpatialAudioObjectRenderStream,
    ISpatialAudioObjectRenderStream_Vtbl,
    0xbab5f473_b423_477b_85f5_b5a332a04153
);
impl core::ops::Deref for ISpatialAudioObjectRenderStream {
    type Target = ISpatialAudioObjectRenderStreamBase;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(
    ISpatialAudioObjectRenderStream,
    windows_core::IUnknown,
    ISpatialAudioObjectRenderStreamBase
);
impl ISpatialAudioObjectRenderStream {
    pub unsafe fn ActivateSpatialAudioObject(
        &self,
        r#type: AudioObjectType,
    ) -> windows_core::Result<ISpatialAudioObject> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ActivateSpatialAudioObject)(
                windows_core::Interface::as_raw(self),
                r#type,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct ISpatialAudioObjectRenderStream_Vtbl {
    pub base__: ISpatialAudioObjectRenderStreamBase_Vtbl,
    pub ActivateSpatialAudioObject: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        AudioObjectType,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for ISpatialAudioObjectRenderStream {}
windows_core::imp::define_interface!(
    ISpatialAudioObjectRenderStreamBase,
    ISpatialAudioObjectRenderStreamBase_Vtbl,
    0xfeaaf403_c1d8_450d_aa05_e0ccee7502a8
);
windows_core::imp::interface_hierarchy!(
    ISpatialAudioObjectRenderStreamBase,
    windows_core::IUnknown
);
impl ISpatialAudioObjectRenderStreamBase {
    pub unsafe fn GetService<T>(&self) -> windows_core::Result<T>
    where
        T: windows_core::Interface,
    {
        let mut result__ = core::ptr::null_mut();
        unsafe {
            (windows_core::Interface::vtable(self).GetService)(
                windows_core::Interface::as_raw(self),
                &T::IID,
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub unsafe fn Start(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Start)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn Stop(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Stop)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn BeginUpdatingAudioObjects(
        &self,
        availabledynamicobjectcount: *mut u32,
        framecountperbuffer: *mut u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).BeginUpdatingAudioObjects)(
                windows_core::Interface::as_raw(self),
                availabledynamicobjectcount as _,
                framecountperbuffer as _,
            )
        }
    }
    pub unsafe fn EndUpdatingAudioObjects(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).EndUpdatingAudioObjects)(
                windows_core::Interface::as_raw(self),
            )
        }
    }
}
#[repr(C)]
pub struct ISpatialAudioObjectRenderStreamBase_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    GetAvailableDynamicObjectCount: usize,
    pub GetService: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub Start: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub Stop: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    Reset: usize,
    pub BeginUpdatingAudioObjects: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut u32,
        *mut u32,
    ) -> windows_core::HRESULT,
    pub EndUpdatingAudioObjects:
        unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
}
impl windows_core::RuntimeName for ISpatialAudioObjectRenderStreamBase {}
windows_core::imp::define_interface!(
    ISpatialAudioObjectRenderStreamNotify,
    ISpatialAudioObjectRenderStreamNotify_Vtbl,
    0xdddf83e6_68d7_4c70_883f_a1836afb4a50
);
windows_core::imp::interface_hierarchy!(
    ISpatialAudioObjectRenderStreamNotify,
    windows_core::IUnknown
);
#[repr(C)]
pub struct ISpatialAudioObjectRenderStreamNotify_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    OnAvailableDynamicObjectCountChange: usize,
}
impl windows_core::RuntimeName for ISpatialAudioObjectRenderStreamNotify {}
windows_core::imp::define_interface!(
    IStorage,
    IStorage_Vtbl,
    0x0000000b_0000_0000_c000_000000000046
);
windows_core::imp::interface_hierarchy!(IStorage, windows_core::IUnknown);
#[repr(C)]
pub struct IStorage_Vtbl {
    pub base__: windows_core::IUnknown_Vtbl,
    CreateStream: usize,
    OpenStream: usize,
    CreateStorage: usize,
    OpenStorage: usize,
    CopyTo: usize,
    MoveElementTo: usize,
    Commit: usize,
    Revert: usize,
    EnumElements: usize,
    DestroyElement: usize,
    RenameElement: usize,
    SetElementTimes: usize,
    SetClass: usize,
    SetStateBits: usize,
    Stat: usize,
}
impl windows_core::RuntimeName for IStorage {}
windows_core::imp::define_interface!(
    IStream,
    IStream_Vtbl,
    0x0000000c_0000_0000_c000_000000000046
);
impl core::ops::Deref for IStream {
    type Target = ISequentialStream;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
windows_core::imp::interface_hierarchy!(IStream, windows_core::IUnknown, ISequentialStream);
impl IStream {
    pub unsafe fn Seek(
        &self,
        dlibmove: i64,
        dworigin: STREAM_SEEK,
        plibnewposition: Option<*mut u64>,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Seek)(
                windows_core::Interface::as_raw(self),
                dlibmove,
                dworigin,
                plibnewposition.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn SetSize(&self, libnewsize: u64) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).SetSize)(
                windows_core::Interface::as_raw(self),
                libnewsize,
            )
        }
    }
    pub unsafe fn CopyTo<P0>(
        &self,
        pstm: P0,
        cb: u64,
        pcbread: Option<*mut u64>,
        pcbwritten: Option<*mut u64>,
    ) -> windows_core::HRESULT
    where
        P0: windows_core::Param<Self>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).CopyTo)(
                windows_core::Interface::as_raw(self),
                pstm.param().abi(),
                cb,
                pcbread.unwrap_or(core::mem::zeroed()) as _,
                pcbwritten.unwrap_or(core::mem::zeroed()) as _,
            )
        }
    }
    pub unsafe fn Commit(&self, grfcommitflags: u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Commit)(
                windows_core::Interface::as_raw(self),
                grfcommitflags,
            )
        }
    }
    pub unsafe fn Revert(&self) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Revert)(windows_core::Interface::as_raw(self))
        }
    }
    pub unsafe fn LockRegion(
        &self,
        liboffset: u64,
        cb: u64,
        dwlocktype: u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).LockRegion)(
                windows_core::Interface::as_raw(self),
                liboffset,
                cb,
                dwlocktype,
            )
        }
    }
    pub unsafe fn UnlockRegion(
        &self,
        liboffset: u64,
        cb: u64,
        dwlocktype: u32,
    ) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).UnlockRegion)(
                windows_core::Interface::as_raw(self),
                liboffset,
                cb,
                dwlocktype,
            )
        }
    }
    pub unsafe fn Stat(&self, pstatstg: *mut STATSTG, grfstatflag: u32) -> windows_core::HRESULT {
        unsafe {
            (windows_core::Interface::vtable(self).Stat)(
                windows_core::Interface::as_raw(self),
                pstatstg as _,
                grfstatflag,
            )
        }
    }
    pub unsafe fn Clone(&self) -> windows_core::Result<Self> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Clone)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IStream_Vtbl {
    pub base__: ISequentialStream_Vtbl,
    pub Seek: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        i64,
        STREAM_SEEK,
        *mut u64,
    ) -> windows_core::HRESULT,
    pub SetSize: unsafe extern "system" fn(*mut core::ffi::c_void, u64) -> windows_core::HRESULT,
    pub CopyTo: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        u64,
        *mut u64,
        *mut u64,
    ) -> windows_core::HRESULT,
    pub Commit: unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> windows_core::HRESULT,
    pub Revert: unsafe extern "system" fn(*mut core::ffi::c_void) -> windows_core::HRESULT,
    pub LockRegion:
        unsafe extern "system" fn(*mut core::ffi::c_void, u64, u64, u32) -> windows_core::HRESULT,
    pub UnlockRegion:
        unsafe extern "system" fn(*mut core::ffi::c_void, u64, u64, u32) -> windows_core::HRESULT,
    pub Stat: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut STATSTG,
        u32,
    ) -> windows_core::HRESULT,
    pub Clone: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
pub trait IStream_Impl: ISequentialStream_Impl {
    fn Seek(
        &self,
        dlibmove: i64,
        dworigin: STREAM_SEEK,
        plibnewposition: *mut u64,
    ) -> windows_core::Result<()>;
    fn SetSize(&self, libnewsize: u64) -> windows_core::Result<()>;
    fn CopyTo(
        &self,
        pstm: windows_core::Ref<IStream>,
        cb: u64,
        pcbread: *mut u64,
        pcbwritten: *mut u64,
    ) -> windows_core::Result<()>;
    fn Commit(&self, grfcommitflags: u32) -> windows_core::Result<()>;
    fn Revert(&self) -> windows_core::Result<()>;
    fn LockRegion(&self, liboffset: u64, cb: u64, dwlocktype: u32) -> windows_core::Result<()>;
    fn UnlockRegion(&self, liboffset: u64, cb: u64, dwlocktype: u32) -> windows_core::Result<()>;
    fn Stat(&self, pstatstg: *mut STATSTG, grfstatflag: u32) -> windows_core::Result<()>;
    fn Clone(&self) -> windows_core::Result<IStream>;
}
impl IStream_Vtbl {
    pub const fn new<Identity: IStream_Impl, const OFFSET: isize>() -> Self {
        unsafe extern "system" fn Seek<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            dlibmove: i64,
            dworigin: STREAM_SEEK,
            plibnewposition: *mut u64,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::Seek(
                    this,
                    core::mem::transmute_copy(&dlibmove),
                    core::mem::transmute_copy(&dworigin),
                    core::mem::transmute_copy(&plibnewposition),
                )
                .into()
            }
        }
        unsafe extern "system" fn SetSize<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            libnewsize: u64,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::SetSize(this, core::mem::transmute_copy(&libnewsize)).into()
            }
        }
        unsafe extern "system" fn CopyTo<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            pstm: *mut core::ffi::c_void,
            cb: u64,
            pcbread: *mut u64,
            pcbwritten: *mut u64,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::CopyTo(
                    this,
                    core::mem::transmute_copy(&pstm),
                    core::mem::transmute_copy(&cb),
                    core::mem::transmute_copy(&pcbread),
                    core::mem::transmute_copy(&pcbwritten),
                )
                .into()
            }
        }
        unsafe extern "system" fn Commit<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            grfcommitflags: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::Commit(this, core::mem::transmute_copy(&grfcommitflags)).into()
            }
        }
        unsafe extern "system" fn Revert<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::Revert(this).into()
            }
        }
        unsafe extern "system" fn LockRegion<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            liboffset: u64,
            cb: u64,
            dwlocktype: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::LockRegion(
                    this,
                    core::mem::transmute_copy(&liboffset),
                    core::mem::transmute_copy(&cb),
                    core::mem::transmute_copy(&dwlocktype),
                )
                .into()
            }
        }
        unsafe extern "system" fn UnlockRegion<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            liboffset: u64,
            cb: u64,
            dwlocktype: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::UnlockRegion(
                    this,
                    core::mem::transmute_copy(&liboffset),
                    core::mem::transmute_copy(&cb),
                    core::mem::transmute_copy(&dwlocktype),
                )
                .into()
            }
        }
        unsafe extern "system" fn Stat<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            pstatstg: *mut STATSTG,
            grfstatflag: u32,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                IStream_Impl::Stat(
                    this,
                    core::mem::transmute_copy(&pstatstg),
                    core::mem::transmute_copy(&grfstatflag),
                )
                .into()
            }
        }
        unsafe extern "system" fn Clone<Identity: IStream_Impl, const OFFSET: isize>(
            this: *mut core::ffi::c_void,
            ppstm: *mut *mut core::ffi::c_void,
        ) -> windows_core::HRESULT {
            unsafe {
                let this: &Identity =
                    &*((this as *const *const ()).offset(OFFSET) as *const Identity);
                match IStream_Impl::Clone(this) {
                    Ok(ok__) => {
                        ppstm.write(core::mem::transmute(ok__));
                        windows_core::HRESULT(0)
                    }
                    Err(err) => err.into(),
                }
            }
        }
        Self {
            base__: ISequentialStream_Vtbl::new::<Identity, OFFSET>(),
            Seek: Seek::<Identity, OFFSET>,
            SetSize: SetSize::<Identity, OFFSET>,
            CopyTo: CopyTo::<Identity, OFFSET>,
            Commit: Commit::<Identity, OFFSET>,
            Revert: Revert::<Identity, OFFSET>,
            LockRegion: LockRegion::<Identity, OFFSET>,
            UnlockRegion: UnlockRegion::<Identity, OFFSET>,
            Stat: Stat::<Identity, OFFSET>,
            Clone: Clone::<Identity, OFFSET>,
        }
    }
    pub fn matches(iid: &windows_core::GUID) -> bool {
        iid == &<IStream as windows_core::Interface>::IID
            || iid == &<ISequentialStream as windows_core::Interface>::IID
    }
}
impl windows_core::RuntimeName for IStream {}
#[repr(C, packed(1))]
#[derive(Clone, Copy, Default)]
pub struct ITEMIDLIST {
    pub mkid: SHITEMID,
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LPARAM(pub isize);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LRESULT(pub isize);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MEMORY_MAPPED_VIEW_ADDRESS(pub *mut core::ffi::c_void);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MFARGB {
    pub rgbBlue: u8,
    pub rgbGreen: u8,
    pub rgbRed: u8,
    pub rgbAlpha: u8,
}
pub const MFASYNC_CALLBACK_QUEUE_STANDARD: u32 = 1;
pub type MFBYTESTREAM_SEEK_ORIGIN = i32;
pub const MFMediaType_Video: windows_core::GUID =
    windows_core::GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
pub const MFSTARTUP_FULL: u32 = 0;
pub const MFT_CATEGORY_VIDEO_DECODER: windows_core::GUID =
    windows_core::GUID::from_u128(0xd6c02d4b_6833_45b4_971a_05a4b04bab91);
pub type MFT_ENUM_FLAG = i32;
pub const MFT_ENUM_FLAG_ALL: MFT_ENUM_FLAG = 63;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MFT_REGISTER_TYPE_INFO {
    pub guidMajorType: windows_core::GUID,
    pub guidSubtype: windows_core::GUID,
}
pub const MFVideoFormat_AV1: windows_core::GUID =
    windows_core::GUID::from_u128(0x31305641_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_H264: windows_core::GUID =
    windows_core::GUID::from_u128(0x34363248_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_H265: windows_core::GUID =
    windows_core::GUID::from_u128(0x35363248_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_HEVC: windows_core::GUID =
    windows_core::GUID::from_u128(0x43564548_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_MPEG2: windows_core::GUID =
    windows_core::GUID::from_u128(0xe06d8026_db46_11cf_b4d1_00805f6cbbea);
pub const MFVideoFormat_Theora: windows_core::GUID =
    windows_core::GUID::from_u128(0x6f656874_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_VP80: windows_core::GUID =
    windows_core::GUID::from_u128(0x30385056_0000_0010_8000_00aa00389b71);
pub const MFVideoFormat_VP90: windows_core::GUID =
    windows_core::GUID::from_u128(0x30395056_0000_0010_8000_00aa00389b71);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MFVideoNormalizedRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
pub const MF_BYTESTREAM_CONTENT_TYPE: windows_core::GUID =
    windows_core::GUID::from_u128(0xfc358289_3cb6_460c_a424_b6681260375a);
pub const MF_E_INVALIDREQUEST: windows_core::HRESULT = windows_core::HRESULT(0xC00D36B2_u32 as _);
pub const MF_E_OPERATION_CANCELLED: windows_core::HRESULT =
    windows_core::HRESULT(0xC00D36ED_u32 as _);
pub const MF_E_UNSUPPORTED_SCHEME: windows_core::HRESULT =
    windows_core::HRESULT(0xC00D36C3_u32 as _);
pub const MF_MEDIA_ENGINE_CALLBACK: windows_core::GUID =
    windows_core::GUID::from_u128(0xc60381b8_83a4_41f8_a3d0_de05076849a9);
pub type MF_MEDIA_ENGINE_CANPLAY = i32;
pub const MF_MEDIA_ENGINE_CANPLAY_MAYBE: MF_MEDIA_ENGINE_CANPLAY = 1;
pub const MF_MEDIA_ENGINE_CANPLAY_NOT_SUPPORTED: MF_MEDIA_ENGINE_CANPLAY = 0;
pub const MF_MEDIA_ENGINE_CANPLAY_PROBABLY: MF_MEDIA_ENGINE_CANPLAY = 2;
pub const MF_MEDIA_ENGINE_DXGI_MANAGER: windows_core::GUID =
    windows_core::GUID::from_u128(0x065702da_1094_486d_8617_ee7cc4ee4648);
pub const MF_MEDIA_ENGINE_EXTENSION: windows_core::GUID =
    windows_core::GUID::from_u128(0x3109fd46_060d_4b62_8dcf_faff811318d2);
pub type MF_MEDIA_ENGINE_PRELOAD = i32;
pub const MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC: MF_MEDIA_ENGINE_PRELOAD = 4;
pub const MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT: windows_core::GUID =
    windows_core::GUID::from_u128(0x5066893c_8cf9_42bc_8b8a_472212e52726);
pub const MF_OBJECT_BYTESTREAM: MF_OBJECT_TYPE = 1;
pub const MF_OBJECT_INVALID: MF_OBJECT_TYPE = 2;
pub const MF_OBJECT_MEDIASOURCE: MF_OBJECT_TYPE = 0;
pub type MF_OBJECT_TYPE = i32;
pub const MF_VERSION: u32 = 131184;
pub const MMDeviceEnumerator: windows_core::GUID =
    windows_core::GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NOTIFICATION_USER_INPUT_DATA {
    pub Key: windows_core::PWSTR,
    pub Value: windows_core::PWSTR,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OSVERSIONINFOW {
    pub dwOSVersionInfoSize: u32,
    pub dwMajorVersion: u32,
    pub dwMinorVersion: u32,
    pub dwBuildNumber: u32,
    pub dwPlatformId: u32,
    pub szCSDVersion: [u16; 128],
}
impl Default for OSVERSIONINFOW {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub type PEEK_MESSAGE_REMOVE_TYPE = u32;
pub const PM_REMOVE: PEEK_MESSAGE_REMOVE_TYPE = 1;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}
pub type PROCESS_LOOPBACK_MODE = i32;
pub const PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE: PROCESS_LOOPBACK_MODE = 1;
pub const PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE: PROCESS_LOOPBACK_MODE = 0;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PROPERTYKEY {
    pub fmtid: windows_core::GUID,
    pub pid: u32,
}
#[repr(C)]
pub struct PROPVARIANT {
    pub Anonymous: PROPVARIANT_0,
}
impl Clone for PROPVARIANT {
    fn clone(&self) -> Self {
        unsafe { core::mem::transmute_copy(self) }
    }
}
impl Default for PROPVARIANT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
pub union PROPVARIANT_0 {
    pub Anonymous: core::mem::ManuallyDrop<PROPVARIANT_0_0>,
    pub decVal: DECIMAL,
}
impl Clone for PROPVARIANT_0 {
    fn clone(&self) -> Self {
        unsafe { core::mem::transmute_copy(self) }
    }
}
impl Default for PROPVARIANT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
pub struct PROPVARIANT_0_0 {
    pub vt: VARENUM,
    pub wReserved1: u16,
    pub wReserved2: u16,
    pub wReserved3: u16,
    pub Anonymous: PROPVARIANT_0_0_0,
}
impl Clone for PROPVARIANT_0_0 {
    fn clone(&self) -> Self {
        unsafe { core::mem::transmute_copy(self) }
    }
}
impl Default for PROPVARIANT_0_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
pub union PROPVARIANT_0_0_0 {
    pub cVal: CHAR,
    pub bVal: u8,
    pub iVal: i16,
    pub uiVal: u16,
    pub lVal: i32,
    pub ulVal: u32,
    pub intVal: i32,
    pub uintVal: u32,
    pub hVal: i64,
    pub uhVal: u64,
    pub fltVal: f32,
    pub dblVal: f64,
    pub boolVal: VARIANT_BOOL,
    pub __OBSOLETE__VARIANT_BOOL: VARIANT_BOOL,
    pub scode: i32,
    pub cyVal: CY,
    pub date: f64,
    pub filetime: FILETIME,
    pub puuid: *mut windows_core::GUID,
    pub pclipdata: *mut CLIPDATA,
    pub bstrVal: core::mem::ManuallyDrop<windows_core::BSTR>,
    pub bstrblobVal: BSTRBLOB,
    pub blob: BLOB,
    pub pszVal: windows_core::PSTR,
    pub pwszVal: windows_core::PWSTR,
    pub punkVal: core::mem::ManuallyDrop<Option<windows_core::IUnknown>>,
    pub pdispVal: core::mem::ManuallyDrop<Option<IDispatch>>,
    pub pStream: core::mem::ManuallyDrop<Option<IStream>>,
    pub pStorage: core::mem::ManuallyDrop<Option<IStorage>>,
    pub pVersionedStream: *mut VERSIONEDSTREAM,
    pub parray: *mut SAFEARRAY,
    pub cac: CAC,
    pub caub: CAUB,
    pub cai: CAI,
    pub caui: CAUI,
    pub cal: CAL,
    pub caul: CAUL,
    pub cah: CAH,
    pub cauh: CAUH,
    pub caflt: CAFLT,
    pub cadbl: CADBL,
    pub cabool: CABOOL,
    pub cascode: CASCODE,
    pub cacy: CACY,
    pub cadate: CADATE,
    pub cafiletime: CAFILETIME,
    pub cauuid: CACLSID,
    pub caclipdata: CACLIPDATA,
    pub cabstr: CABSTR,
    pub cabstrblob: CABSTRBLOB,
    pub calpstr: CALPSTR,
    pub calpwstr: CALPWSTR,
    pub capropvar: CAPROPVARIANT,
    pub pcVal: windows_core::PSTR,
    pub pbVal: *mut u8,
    pub piVal: *mut i16,
    pub puiVal: *mut u16,
    pub plVal: *mut i32,
    pub pulVal: *mut u32,
    pub pintVal: *mut i32,
    pub puintVal: *mut u32,
    pub pfltVal: *mut f32,
    pub pdblVal: *mut f64,
    pub pboolVal: *mut VARIANT_BOOL,
    pub pdecVal: *mut DECIMAL,
    pub pscode: *mut i32,
    pub pcyVal: *mut CY,
    pub pdate: *mut f64,
    pub pbstrVal: *mut windows_core::BSTR,
    pub ppunkVal: *mut Option<windows_core::IUnknown>,
    pub ppdispVal: *mut Option<IDispatch>,
    pub pparray: *mut *mut SAFEARRAY,
    pub pvarVal: *mut PROPVARIANT,
}
impl Clone for PROPVARIANT_0_0_0 {
    fn clone(&self) -> Self {
        unsafe { core::mem::transmute_copy(self) }
    }
}
impl Default for PROPVARIANT_0_0_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
pub type REGCLS = i32;
pub const REGCLS_MULTIPLEUSE: REGCLS = 1;
pub const RO_INIT_MULTITHREADED: RO_INIT_TYPE = 1;
pub type RO_INIT_TYPE = i32;
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SAFEARRAY {
    pub cDims: u16,
    pub fFeatures: ADVANCED_FEATURE_FLAGS,
    pub cbElements: u32,
    pub cLocks: u32,
    pub pvData: *mut core::ffi::c_void,
    pub rgsabound: [SAFEARRAYBOUND; 1],
}
impl Default for SAFEARRAY {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SAFEARRAYBOUND {
    pub cElements: u32,
    pub lLbound: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: u32,
    pub lpSecurityDescriptor: *mut core::ffi::c_void,
    pub bInheritHandle: windows_core::BOOL,
}
#[repr(C, packed(1))]
#[derive(Clone, Copy)]
pub struct SHITEMID {
    pub cb: u16,
    pub abID: [u8; 1],
}
impl Default for SHITEMID {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub type SHOW_WINDOW_CMD = i32;
pub const SPTLAUDCLNT_E_NO_MORE_OBJECTS: windows_core::HRESULT =
    windows_core::HRESULT(0x88890103_u32 as _);
pub type STATFLAG = i32;
pub const STATFLAG_NONAME: STATFLAG = 1;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct STATSTG {
    pub pwcsName: windows_core::PWSTR,
    pub r#type: u32,
    pub cbSize: u64,
    pub mtime: FILETIME,
    pub ctime: FILETIME,
    pub atime: FILETIME,
    pub grfMode: STGM,
    pub grfLocksSupported: u32,
    pub clsid: windows_core::GUID,
    pub grfStateBits: u32,
    pub reserved: u32,
}
pub type STGC = i32;
pub const STGC_DEFAULT: STGC = 0;
pub type STGM = u32;
pub const STGM_READ: STGM = 0;
pub type STGTY = i32;
pub const STGTY_STREAM: STGTY = 2;
pub const STG_E_ACCESSDENIED: windows_core::HRESULT = windows_core::HRESULT(0x80030005_u32 as _);
pub const STG_E_INVALIDFUNCTION: windows_core::HRESULT = windows_core::HRESULT(0x80030001_u32 as _);
pub type STREAM_SEEK = u32;
pub const STREAM_SEEK_CUR: STREAM_SEEK = 1;
pub const STREAM_SEEK_END: STREAM_SEEK = 2;
pub const STREAM_SEEK_SET: STREAM_SEEK = 0;
pub const SW_SHOWNORMAL: SHOW_WINDOW_CMD = 1;
pub const S_FALSE: windows_core::HRESULT = windows_core::HRESULT(0x1_u32 as _);
pub const S_OK: windows_core::HRESULT = windows_core::HRESULT(0x0_u32 as _);
#[repr(C, packed(1))]
#[derive(Default)]
pub struct SpatialAudioObjectRenderStreamActivationParams {
    pub ObjectFormat: *mut WAVEFORMATEX,
    pub StaticObjectTypeMask: AudioObjectType,
    pub MinDynamicObjectCount: u32,
    pub MaxDynamicObjectCount: u32,
    pub Category: AUDIO_STREAM_CATEGORY,
    pub EventHandle: HANDLE,
    pub NotifyObject: core::mem::ManuallyDrop<Option<ISpatialAudioObjectRenderStreamNotify>>,
}
pub const TSF_DELETE_RECYCLE_IF_POSSIBLE: _TRANSFER_SOURCE_FLAGS = 128;
pub type VARENUM = u16;
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VARIANT_BOOL(pub i16);
#[repr(C)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VERSIONEDSTREAM {
    pub guidVersion: windows_core::GUID,
    pub pStream: core::mem::ManuallyDrop<Option<IStream>>,
}
pub const VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK: windows_core::PCWSTR =
    windows_core::w!("VAD\\Process_Loopback");
pub const VT_BLOB: VARENUM = 65;
pub type WAIT_EVENT = u32;
pub const WAIT_FAILED: WAIT_EVENT = 4294967295;
pub const WAIT_OBJECT_0: WAIT_EVENT = 0;
#[repr(C, packed(1))]
#[derive(Clone, Copy, Default)]
pub struct WAVEFORMATEX {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}
pub type WIN32_ERROR = u32;
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WPARAM(pub usize);
pub type _AUDCLNT_BUFFERFLAGS = i32;
pub type _TRANSFER_SOURCE_FLAGS = i32;
pub const eAll: EDataFlow = 2;
pub const eCapture: EDataFlow = 1;
pub const eCommunications: ERole = 2;
pub const eConsole: ERole = 0;
pub const eMultimedia: ERole = 1;
pub const eRender: EDataFlow = 0;
