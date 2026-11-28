windows_link::link!("kernel32.dll" "system" fn AddVectoredExceptionHandler(first : u32, handler : PVECTORED_EXCEPTION_HANDLER) -> *mut core::ffi::c_void);
windows_link::link!("user32.dll" "system" fn AllowSetForegroundWindow(dwprocessid : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn AssignProcessToJobObject(hjob : HANDLE, hprocess : HANDLE) -> BOOL);
windows_link::link!("bcrypt.dll" "system" fn BCryptCloseAlgorithmProvider(halgorithm : BCRYPT_ALG_HANDLE, dwflags : u32) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptDecrypt(hkey : BCRYPT_KEY_HANDLE, pbinput : *const u8, cbinput : u32, ppaddinginfo : *const core::ffi::c_void, pbiv : *mut u8, cbiv : u32, pboutput : *mut u8, cboutput : u32, pcbresult : *mut u32, dwflags : BCRYPT_FLAGS) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptDestroyKey(hkey : BCRYPT_KEY_HANDLE) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptEncrypt(hkey : BCRYPT_KEY_HANDLE, pbinput : *const u8, cbinput : u32, ppaddinginfo : *const core::ffi::c_void, pbiv : *mut u8, cbiv : u32, pboutput : *mut u8, cboutput : u32, pcbresult : *mut u32, dwflags : BCRYPT_FLAGS) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptGenRandom(halgorithm : BCRYPT_ALG_HANDLE, pbbuffer : *mut u8, cbbuffer : u32, dwflags : BCRYPTGENRANDOM_FLAGS) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptGenerateSymmetricKey(halgorithm : BCRYPT_ALG_HANDLE, phkey : *mut BCRYPT_KEY_HANDLE, pbkeyobject : *mut u8, cbkeyobject : u32, pbsecret : *const u8, cbsecret : u32, dwflags : u32) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptOpenAlgorithmProvider(phalgorithm : *mut BCRYPT_ALG_HANDLE, pszalgid : PCWSTR, pszimplementation : PCWSTR, dwflags : BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS) -> NTSTATUS);
windows_link::link!("bcrypt.dll" "system" fn BCryptSetProperty(hobject : BCRYPT_HANDLE, pszproperty : PCWSTR, pbinput : *const u8, cbinput : u32, dwflags : u32) -> NTSTATUS);
windows_link::link!("user32.dll" "system" fn CallNextHookEx(hhk : HHOOK, ncode : i32, wparam : WPARAM, lparam : LPARAM) -> LRESULT);
windows_link::link!("kernel32.dll" "system" fn CancelIoEx(hfile : HANDLE, lpoverlapped : *const OVERLAPPED) -> BOOL);
windows_link::link!("user32.dll" "system" fn CloseClipboard() -> BOOL);
windows_link::link!("kernel32.dll" "system" fn CloseHandle(hobject : HANDLE) -> BOOL);
windows_link::link!("ole32.dll" "system" fn CoGetApartmentType(papttype : *mut APTTYPE, paptqualifier : *mut APTTYPEQUALIFIER) -> HRESULT);
windows_link::link!("ole32.dll" "system" fn CoTaskMemFree(pv : *const core::ffi::c_void));
windows_link::link!("kernel32.dll" "system" fn ConnectNamedPipe(hnamedpipe : HANDLE, lpoverlapped : *mut OVERLAPPED) -> BOOL);
windows_link::link!("advapi32.dll" "system" fn ConvertSidToStringSidW(sid : PSID, stringsid : *mut PWSTR) -> BOOL);
windows_link::link!("advapi32.dll" "system" fn ConvertStringSecurityDescriptorToSecurityDescriptorW(stringsecuritydescriptor : PCWSTR, stringsdrevision : u32, securitydescriptor : *mut PSECURITY_DESCRIPTOR, securitydescriptorsize : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn CreateEventW(lpeventattributes : *const SECURITY_ATTRIBUTES, bmanualreset : BOOL, binitialstate : BOOL, lpname : PCWSTR) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn CreateFileW(lpfilename : PCWSTR, dwdesiredaccess : u32, dwsharemode : FILE_SHARE_MODE, lpsecurityattributes : *const SECURITY_ATTRIBUTES, dwcreationdisposition : FILE_CREATION_DISPOSITION, dwflagsandattributes : FILE_FLAGS_AND_ATTRIBUTES, htemplatefile : HANDLE) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn CreateJobObjectW(lpjobattributes : *const SECURITY_ATTRIBUTES, lpname : PCWSTR) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn CreateMutexW(lpmutexattributes : *const SECURITY_ATTRIBUTES, binitialowner : BOOL, lpname : PCWSTR) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn CreateNamedPipeW(lpname : PCWSTR, dwopenmode : FILE_FLAGS_AND_ATTRIBUTES, dwpipemode : NAMED_PIPE_MODE, nmaxinstances : u32, noutbuffersize : u32, ninbuffersize : u32, ndefaulttimeout : u32, lpsecurityattributes : *const SECURITY_ATTRIBUTES) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn CreatePipe(hreadpipe : *mut HANDLE, hwritepipe : *mut HANDLE, lppipeattributes : *const SECURITY_ATTRIBUTES, nsize : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn CreateProcessW(lpapplicationname : PCWSTR, lpcommandline : PWSTR, lpprocessattributes : *const SECURITY_ATTRIBUTES, lpthreadattributes : *const SECURITY_ATTRIBUTES, binherithandles : BOOL, dwcreationflags : PROCESS_CREATION_FLAGS, lpenvironment : *const core::ffi::c_void, lpcurrentdirectory : PCWSTR, lpstartupinfo : *const STARTUPINFOW, lpprocessinformation : *mut PROCESS_INFORMATION) -> BOOL);
windows_link::link!("crypt32.dll" "system" fn CryptProtectData(pdatain : *const CRYPT_INTEGER_BLOB, szdatadescr : PCWSTR, poptionalentropy : *const CRYPT_INTEGER_BLOB, pvreserved : *const core::ffi::c_void, ppromptstruct : *const CRYPTPROTECT_PROMPTSTRUCT, dwflags : u32, pdataout : *mut CRYPT_INTEGER_BLOB) -> BOOL);
windows_link::link!("crypt32.dll" "system" fn CryptUnprotectData(pdatain : *const CRYPT_INTEGER_BLOB, ppszdatadescr : *mut PWSTR, poptionalentropy : *const CRYPT_INTEGER_BLOB, pvreserved : *const core::ffi::c_void, ppromptstruct : *const CRYPTPROTECT_PROMPTSTRUCT, dwflags : u32, pdataout : *mut CRYPT_INTEGER_BLOB) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn DeleteProcThreadAttributeList(lpattributelist : LPPROC_THREAD_ATTRIBUTE_LIST));
windows_link::link!("kernel32.dll" "system" fn DisconnectNamedPipe(hnamedpipe : HANDLE) -> BOOL);
windows_link::link!("user32.dll" "system" fn DispatchMessageW(lpmsg : *const MSG) -> LRESULT);
windows_link::link!("kernel32.dll" "system" fn DuplicateHandle(hsourceprocesshandle : HANDLE, hsourcehandle : HANDLE, htargetprocesshandle : HANDLE, lptargethandle : *mut HANDLE, dwdesiredaccess : u32, binherithandle : BOOL, dwoptions : DUPLICATE_HANDLE_OPTIONS) -> BOOL);
windows_link::link!("user32.dll" "system" fn EmptyClipboard() -> BOOL);
windows_link::link!("kernel32.dll" "system" fn FlushFileBuffers(hfile : HANDLE) -> BOOL);
windows_link::link!("iphlpapi.dll" "system" fn GetAdaptersAddresses(family : u32, flags : GET_ADAPTERS_ADDRESSES_FLAGS, reserved : *const core::ffi::c_void, adapteraddresses : *mut IP_ADAPTER_ADDRESSES_LH, sizepointer : *mut u32) -> u32);
windows_link::link!("user32.dll" "system" fn GetAsyncKeyState(vkey : i32) -> i16);
windows_link::link!("user32.dll" "system" fn GetClipboardData(uformat : u32) -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn GetComputerNameExW(nametype : COMPUTER_NAME_FORMAT, lpbuffer : PWSTR, nsize : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetCurrentApplicationUserModelId(applicationusermodelidlength : *mut u32, applicationusermodelid : PWSTR) -> WIN32_ERROR);
windows_link::link!("kernel32.dll" "system" fn GetCurrentPackageFamilyName(packagefamilynamelength : *mut u32, packagefamilyname : PWSTR) -> WIN32_ERROR);
windows_link::link!("kernel32.dll" "system" fn GetCurrentProcess() -> HANDLE);
windows_link::link!("kernel32.dll" "system" fn GetCurrentProcessId() -> u32);
windows_link::link!("kernel32.dll" "system" fn GetCurrentThreadId() -> u32);
windows_link::link!("kernel32.dll" "system" fn GetDiskFreeSpaceExW(lpdirectoryname : PCWSTR, lpfreebytesavailabletocaller : *mut u64, lptotalnumberofbytes : *mut u64, lptotalnumberoffreebytes : *mut u64) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetExitCodeProcess(hprocess : HANDLE, lpexitcode : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetFileInformationByHandle(hfile : HANDLE, lpfileinformation : *mut BY_HANDLE_FILE_INFORMATION) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetFinalPathNameByHandleW(hfile : HANDLE, lpszfilepath : PWSTR, cchfilepath : u32, dwflags : GETFINALPATHNAMEBYHANDLE_FLAGS) -> u32);
windows_link::link!("user32.dll" "system" fn GetForegroundWindow() -> HWND);
windows_link::link!("kernel32.dll" "system" fn GetHandleInformation(hobject : HANDLE, lpdwflags : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetLastError() -> WIN32_ERROR);
windows_link::link!("user32.dll" "system" fn GetMessageW(lpmsg : *mut MSG, hwnd : HWND, wmsgfiltermin : u32, wmsgfiltermax : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetModuleHandleW(lpmodulename : PCWSTR) -> HMODULE);
windows_link::link!("kernel32.dll" "system" fn GetNamedPipeClientProcessId(pipe : HANDLE, clientprocessid : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetNamedPipeServerProcessId(pipe : HANDLE, serverprocessid : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetOverlappedResult(hfile : HANDLE, lpoverlapped : *const OVERLAPPED, lpnumberofbytestransferred : *mut u32, bwait : BOOL) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GetProcAddress(hmodule : HMODULE, lpprocname : PCSTR) -> FARPROC);
windows_link::link!("user32.dll" "system" fn GetRawInputDeviceInfoW(hdevice : HANDLE, uicommand : RAW_INPUT_DEVICE_INFO_COMMAND, pdata : *mut core::ffi::c_void, pcbsize : *mut u32) -> u32);
windows_link::link!("user32.dll" "system" fn GetRawInputDeviceList(prawinputdevicelist : *mut RAWINPUTDEVICELIST, puinumdevices : *mut u32, cbsize : u32) -> u32);
windows_link::link!("kernel32.dll" "system" fn GetShortPathNameW(lpszlongpath : PCWSTR, lpszshortpath : PWSTR, cchbuffer : u32) -> u32);
windows_link::link!("kernel32.dll" "system" fn GetSystemDirectoryW(lpbuffer : PWSTR, usize : u32) -> u32);
windows_link::link!("user32.dll" "system" fn GetSystemMetrics(nindex : SYSTEM_METRICS_INDEX) -> i32);
windows_link::link!("kernel32.dll" "system" fn GetTempPath2W(bufferlength : u32, buffer : PWSTR) -> u32);
windows_link::link!("advapi32.dll" "system" fn GetTokenInformation(tokenhandle : HANDLE, tokeninformationclass : TOKEN_INFORMATION_CLASS, tokeninformation : *mut core::ffi::c_void, tokeninformationlength : u32, returnlength : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn GlobalAlloc(uflags : GLOBAL_ALLOC_FLAGS, dwbytes : usize) -> HGLOBAL);
windows_link::link!("kernel32.dll" "system" fn GlobalFree(hmem : HGLOBAL) -> HGLOBAL);
windows_link::link!("kernel32.dll" "system" fn GlobalLock(hmem : HGLOBAL) -> *mut core::ffi::c_void);
windows_link::link!("kernel32.dll" "system" fn GlobalSize(hmem : HGLOBAL) -> usize);
windows_link::link!("kernel32.dll" "system" fn GlobalUnlock(hmem : HGLOBAL) -> BOOL);
windows_link::link!("hid.dll" "system" fn HidD_FreePreparsedData(preparseddata : PHIDP_PREPARSED_DATA) -> BOOLEAN);
windows_link::link!("hid.dll" "system" fn HidD_GetFeature(hiddeviceobject : HANDLE, reportbuffer : *mut core::ffi::c_void, reportbufferlength : u32) -> BOOLEAN);
windows_link::link!("hid.dll" "system" fn HidD_GetPreparsedData(hiddeviceobject : HANDLE, preparseddata : *mut PHIDP_PREPARSED_DATA) -> BOOLEAN);
windows_link::link!("hid.dll" "system" fn HidD_SetFeature(hiddeviceobject : HANDLE, reportbuffer : *const core::ffi::c_void, reportbufferlength : u32) -> BOOLEAN);
windows_link::link!("hid.dll" "system" fn HidP_GetCaps(preparseddata : PHIDP_PREPARSED_DATA, capabilities : *mut HIDP_CAPS) -> NTSTATUS);
windows_link::link!("kernel32.dll" "system" fn InitializeProcThreadAttributeList(lpattributelist : LPPROC_THREAD_ATTRIBUTE_LIST, dwattributecount : u32, dwflags : u32, lpsize : *mut usize) -> BOOL);
windows_link::link!("user32.dll" "system" fn IsClipboardFormatAvailable(format : u32) -> BOOL);
windows_link::link!("user32.dll" "system" fn IsIconic(hwnd : HWND) -> BOOL);
windows_link::link!("user32.dll" "system" fn IsWindowVisible(hwnd : HWND) -> BOOL);
windows_link::link!("user32.dll" "system" fn KillTimer(hwnd : HWND, uidevent : usize) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn LoadLibraryExW(lplibfilename : PCWSTR, hfile : HANDLE, dwflags : LOAD_LIBRARY_FLAGS) -> HMODULE);
windows_link::link!("kernel32.dll" "system" fn LocalFree(hmem : HLOCAL) -> HLOCAL);
windows_link::link!("dbghelp.dll" "system" fn MiniDumpWriteDump(hprocess : HANDLE, processid : u32, hfile : HANDLE, dumptype : MINIDUMP_TYPE, exceptionparam : *const MINIDUMP_EXCEPTION_INFORMATION, userstreamparam : *const MINIDUMP_USER_STREAM_INFORMATION, callbackparam : *const MINIDUMP_CALLBACK_INFORMATION) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn MoveFileExW(lpexistingfilename : PCWSTR, lpnewfilename : PCWSTR, dwflags : MOVE_FILE_FLAGS) -> BOOL);
windows_link::link!("user32.dll" "system" fn OpenClipboard(hwndnewowner : HWND) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn OpenProcess(dwdesiredaccess : PROCESS_ACCESS_RIGHTS, binherithandle : BOOL, dwprocessid : u32) -> HANDLE);
windows_link::link!("advapi32.dll" "system" fn OpenProcessToken(processhandle : HANDLE, desiredaccess : TOKEN_ACCESS_MASK, tokenhandle : *mut HANDLE) -> BOOL);
windows_link::link!("user32.dll" "system" fn PeekMessageW(lpmsg : *mut MSG, hwnd : HWND, wmsgfiltermin : u32, wmsgfiltermax : u32, wremovemsg : PEEK_MESSAGE_REMOVE_TYPE) -> BOOL);
windows_link::link!("user32.dll" "system" fn PostThreadMessageW(idthread : u32, msg : u32, wparam : WPARAM, lparam : LPARAM) -> BOOL);
windows_link::link!("user32.dll" "system" fn PrintWindow(hwnd : HWND, hdcblt : HDC, nflags : PRINT_WINDOW_FLAGS) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn ProcessIdToSessionId(dwprocessid : u32, psessionid : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn QueryFullProcessImageNameW(hprocess : HANDLE, dwflags : PROCESS_NAME_FORMAT, lpexename : PWSTR, lpdwsize : *mut u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn ReadFile(hfile : HANDLE, lpbuffer : *mut u8, nnumberofbytestoread : u32, lpnumberofbytesread : *mut u32, lpoverlapped : *mut OVERLAPPED) -> BOOL);
windows_link::link!("advapi32.dll" "system" fn RegCloseKey(hkey : HKEY) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegCreateKeyExW(hkey : HKEY, lpsubkey : PCWSTR, reserved : u32, lpclass : PCWSTR, dwoptions : REG_OPEN_CREATE_OPTIONS, samdesired : REG_SAM_FLAGS, lpsecurityattributes : *const SECURITY_ATTRIBUTES, phkresult : *mut HKEY, lpdwdisposition : *mut REG_CREATE_KEY_DISPOSITION) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegDeleteTreeW(hkey : HKEY, lpsubkey : PCWSTR) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegEnumKeyExW(hkey : HKEY, dwindex : u32, lpname : PWSTR, lpcchname : *mut u32, lpreserved : *const u32, lpclass : PWSTR, lpcchclass : *mut u32, lpftlastwritetime : *mut FILETIME) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegGetValueW(hkey : HKEY, lpsubkey : PCWSTR, lpvalue : PCWSTR, dwflags : REG_ROUTINE_FLAGS, pdwtype : *mut REG_VALUE_TYPE, pvdata : *mut core::ffi::c_void, pcbdata : *mut u32) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegNotifyChangeKeyValue(hkey : HKEY, bwatchsubtree : BOOL, dwnotifyfilter : REG_NOTIFY_FILTER, hevent : HANDLE, fasynchronous : BOOL) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegOpenKeyExW(hkey : HKEY, lpsubkey : PCWSTR, uloptions : u32, samdesired : REG_SAM_FLAGS, phkresult : *mut HKEY) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegQueryValueExW(hkey : HKEY, lpvaluename : PCWSTR, lpreserved : *const u32, lptype : *mut REG_VALUE_TYPE, lpdata : *mut u8, lpcbdata : *mut u32) -> WIN32_ERROR);
windows_link::link!("advapi32.dll" "system" fn RegSetValueExW(hkey : HKEY, lpvaluename : PCWSTR, reserved : u32, dwtype : REG_VALUE_TYPE, lpdata : *const u8, cbdata : u32) -> WIN32_ERROR);
windows_link::link!("user32.dll" "system" fn RegisterClipboardFormatW(lpszformat : PCWSTR) -> u32);
windows_link::link!("kernel32.dll" "system" fn ReplaceFileW(lpreplacedfilename : PCWSTR, lpreplacementfilename : PCWSTR, lpbackupfilename : PCWSTR, dwreplaceflags : REPLACE_FILE_FLAGS, lpexclude : *const core::ffi::c_void, lpreserved : *const core::ffi::c_void) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn ResetEvent(hevent : HANDLE) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn ResumeThread(hthread : HANDLE) -> u32);
windows_link::link!("ntdll.dll" "system" fn RtlGetVersion(lpversioninformation : *mut OSVERSIONINFOW) -> NTSTATUS);
windows_link::link!("shell32.dll" "system" fn SHChangeNotify(weventid : i32, uflags : SHCNF_FLAGS, dwitem1 : *const core::ffi::c_void, dwitem2 : *const core::ffi::c_void));
windows_link::link!("shell32.dll" "system" fn SHGetKnownFolderPath(rfid : *const GUID, dwflags : u32, htoken : HANDLE, ppszpath : *mut PWSTR) -> HRESULT);
windows_link::link!("user32.dll" "system" fn SendInput(cinputs : u32, pinputs : *const INPUT, cbsize : i32) -> u32);
windows_link::link!("user32.dll" "system" fn SetClipboardData(uformat : u32, hmem : HANDLE) -> HANDLE);
windows_link::link!("user32.dll" "system" fn SetForegroundWindow(hwnd : HWND) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn SetInformationJobObject(hjob : HANDLE, jobobjectinformationclass : JOBOBJECTINFOCLASS, lpjobobjectinformation : *const core::ffi::c_void, cbjobobjectinformationlength : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn SetLastError(dwerrcode : WIN32_ERROR));
windows_link::link!("kernel32.dll" "system" fn SetNamedPipeHandleState(hnamedpipe : HANDLE, lpmode : *const NAMED_PIPE_MODE, lpmaxcollectioncount : *const u32, lpcollectdatatimeout : *const u32) -> BOOL);
windows_link::link!("user32.dll" "system" fn SetTimer(hwnd : HWND, nidevent : usize, uelapse : u32, lptimerfunc : TIMERPROC) -> usize);
windows_link::link!("kernel32.dll" "system" fn SetUnhandledExceptionFilter(lptoplevelexceptionfilter : LPTOP_LEVEL_EXCEPTION_FILTER) -> LPTOP_LEVEL_EXCEPTION_FILTER);
windows_link::link!("user32.dll" "system" fn SetWindowsHookExW(idhook : WINDOWS_HOOK_ID, lpfn : HOOKPROC, hmod : HINSTANCE, dwthreadid : u32) -> HHOOK);
windows_link::link!("shell32.dll" "system" fn ShellExecuteExW(pexecinfo : *mut SHELLEXECUTEINFOW) -> BOOL);
windows_link::link!("shell32.dll" "system" fn ShellExecuteW(hwnd : HWND, lpoperation : PCWSTR, lpfile : PCWSTR, lpparameters : PCWSTR, lpdirectory : PCWSTR, nshowcmd : SHOW_WINDOW_CMD) -> HINSTANCE);
windows_link::link!("user32.dll" "system" fn ShowWindowAsync(hwnd : HWND, ncmdshow : SHOW_WINDOW_CMD) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn TerminateJobObject(hjob : HANDLE, uexitcode : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn TerminateProcess(hprocess : HANDLE, uexitcode : u32) -> BOOL);
windows_link::link!("user32.dll" "system" fn TranslateMessage(lpmsg : *const MSG) -> BOOL);
windows_link::link!("user32.dll" "system" fn UnhookWindowsHookEx(hhk : HHOOK) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn UpdateProcThreadAttribute(lpattributelist : LPPROC_THREAD_ATTRIBUTE_LIST, dwflags : u32, attribute : usize, lpvalue : *const core::ffi::c_void, cbsize : usize, lppreviousvalue : *mut core::ffi::c_void, lpreturnsize : *const usize) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn WaitForMultipleObjects(ncount : u32, lphandles : *const HANDLE, bwaitall : BOOL, dwmilliseconds : u32) -> WAIT_EVENT);
windows_link::link!("kernel32.dll" "system" fn WaitForSingleObject(hhandle : HANDLE, dwmilliseconds : u32) -> WAIT_EVENT);
windows_link::link!("kernel32.dll" "system" fn WaitNamedPipeW(lpnamedpipename : PCWSTR, ntimeout : u32) -> BOOL);
windows_link::link!("kernel32.dll" "system" fn WriteFile(hfile : HANDLE, lpbuffer : *const u8, nnumberofbytestowrite : u32, lpnumberofbyteswritten : *mut u32, lpoverlapped : *mut OVERLAPPED) -> BOOL);
pub type ADDRESS_FAMILY = u16;
pub type APTTYPE = i32;
pub type APTTYPEQUALIFIER = i32;
pub const APTTYPE_MAINSTA: APTTYPE = 3;
pub const APTTYPE_STA: APTTYPE = 0;
#[repr(C)]
#[derive(Clone, Copy)]
pub union ARM64_NT_NEON128 {
    pub Anonymous: ARM64_NT_NEON128_0,
    pub D: [f64; 2],
    pub S: [f32; 4],
    pub H: [u16; 8],
    pub B: [u8; 16],
}
impl Default for ARM64_NT_NEON128 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ARM64_NT_NEON128_0 {
    pub Low: u64,
    pub High: i64,
}
pub type BCRYPTGENRANDOM_FLAGS = u32;
pub const BCRYPT_AES_ALGORITHM: PCWSTR = [65, 69, 83, 0].as_ptr();
pub type BCRYPT_ALG_HANDLE = *mut core::ffi::c_void;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
    pub cbSize: u32,
    pub dwInfoVersion: u32,
    pub pbNonce: *mut u8,
    pub cbNonce: u32,
    pub pbAuthData: *mut u8,
    pub cbAuthData: u32,
    pub pbTag: *mut u8,
    pub cbTag: u32,
    pub pbMacContext: *mut u8,
    pub cbMacContext: u32,
    pub cbAAD: u32,
    pub cbData: u64,
    pub dwFlags: u32,
}
pub const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION: u32 = 1;
pub const BCRYPT_CHAINING_MODE: PCWSTR =
    [67, 104, 97, 105, 110, 105, 110, 103, 77, 111, 100, 101, 0].as_ptr();
pub const BCRYPT_CHAIN_MODE_GCM: PCWSTR = [
    67, 104, 97, 105, 110, 105, 110, 103, 77, 111, 100, 101, 71, 67, 77, 0,
]
.as_ptr();
pub type BCRYPT_FLAGS = u32;
pub type BCRYPT_HANDLE = *mut core::ffi::c_void;
pub type BCRYPT_KEY_HANDLE = *mut core::ffi::c_void;
pub type BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS = u32;
pub const BCRYPT_USE_SYSTEM_PREFERRED_RNG: BCRYPTGENRANDOM_FLAGS = 2;
pub type BOOL = i32;
pub type BOOLEAN = u8;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BY_HANDLE_FILE_INFORMATION {
    pub dwFileAttributes: u32,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub dwVolumeSerialNumber: u32,
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
    pub nNumberOfLinks: u32,
    pub nFileIndexHigh: u32,
    pub nFileIndexLow: u32,
}
pub type CHAR = i8;
pub type COMPUTER_NAME_FORMAT = i32;
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy)]
pub struct CONTEXT {
    pub ContextFlags: CONTEXT_FLAGS,
    pub Dr0: u32,
    pub Dr1: u32,
    pub Dr2: u32,
    pub Dr3: u32,
    pub Dr6: u32,
    pub Dr7: u32,
    pub FloatSave: FLOATING_SAVE_AREA,
    pub SegGs: u32,
    pub SegFs: u32,
    pub SegEs: u32,
    pub SegDs: u32,
    pub Edi: u32,
    pub Esi: u32,
    pub Ebx: u32,
    pub Edx: u32,
    pub Ecx: u32,
    pub Eax: u32,
    pub Ebp: u32,
    pub Eip: u32,
    pub SegCs: u32,
    pub EFlags: u32,
    pub Esp: u32,
    pub SegSs: u32,
    pub ExtendedRegisters: [u8; 512],
}
#[cfg(target_arch = "x86")]
impl Default for CONTEXT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub struct CONTEXT {
    pub P1Home: u64,
    pub P2Home: u64,
    pub P3Home: u64,
    pub P4Home: u64,
    pub P5Home: u64,
    pub P6Home: u64,
    pub ContextFlags: CONTEXT_FLAGS,
    pub MxCsr: u32,
    pub SegCs: u16,
    pub SegDs: u16,
    pub SegEs: u16,
    pub SegFs: u16,
    pub SegGs: u16,
    pub SegSs: u16,
    pub EFlags: u32,
    pub Dr0: u64,
    pub Dr1: u64,
    pub Dr2: u64,
    pub Dr3: u64,
    pub Dr6: u64,
    pub Dr7: u64,
    pub Rax: u64,
    pub Rcx: u64,
    pub Rdx: u64,
    pub Rbx: u64,
    pub Rsp: u64,
    pub Rbp: u64,
    pub Rsi: u64,
    pub Rdi: u64,
    pub R8: u64,
    pub R9: u64,
    pub R10: u64,
    pub R11: u64,
    pub R12: u64,
    pub R13: u64,
    pub R14: u64,
    pub R15: u64,
    pub Rip: u64,
    pub Anonymous: CONTEXT_0,
    pub VectorRegister: [M128A; 26],
    pub VectorControl: u64,
    pub DebugControl: u64,
    pub LastBranchToRip: u64,
    pub LastBranchFromRip: u64,
    pub LastExceptionToRip: u64,
    pub LastExceptionFromRip: u64,
}
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
impl Default for CONTEXT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub union CONTEXT_0 {
    pub FltSave: XSAVE_FORMAT,
    pub Anonymous: CONTEXT_0_0,
}
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
impl Default for CONTEXT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub struct CONTEXT_0_0 {
    pub Header: [M128A; 2],
    pub Legacy: [M128A; 8],
    pub Xmm0: M128A,
    pub Xmm1: M128A,
    pub Xmm2: M128A,
    pub Xmm3: M128A,
    pub Xmm4: M128A,
    pub Xmm5: M128A,
    pub Xmm6: M128A,
    pub Xmm7: M128A,
    pub Xmm8: M128A,
    pub Xmm9: M128A,
    pub Xmm10: M128A,
    pub Xmm11: M128A,
    pub Xmm12: M128A,
    pub Xmm13: M128A,
    pub Xmm14: M128A,
    pub Xmm15: M128A,
}
#[cfg(any(target_arch = "arm64ec", target_arch = "x86_64"))]
impl Default for CONTEXT_0_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub struct CONTEXT {
    pub ContextFlags: CONTEXT_FLAGS,
    pub Cpsr: u32,
    pub Anonymous: CONTEXT_0,
    pub Sp: u64,
    pub Pc: u64,
    pub V: [ARM64_NT_NEON128; 32],
    pub Fpcr: u32,
    pub Fpsr: u32,
    pub Bcr: [u32; 8],
    pub Bvr: [u64; 8],
    pub Wcr: [u32; 2],
    pub Wvr: [u64; 2],
}
#[cfg(target_arch = "aarch64")]
impl Default for CONTEXT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub union CONTEXT_0 {
    pub Anonymous: CONTEXT_0_0,
    pub X: [u64; 31],
}
#[cfg(target_arch = "aarch64")]
impl Default for CONTEXT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy, Default)]
pub struct CONTEXT_0_0 {
    pub X0: u64,
    pub X1: u64,
    pub X2: u64,
    pub X3: u64,
    pub X4: u64,
    pub X5: u64,
    pub X6: u64,
    pub X7: u64,
    pub X8: u64,
    pub X9: u64,
    pub X10: u64,
    pub X11: u64,
    pub X12: u64,
    pub X13: u64,
    pub X14: u64,
    pub X15: u64,
    pub X16: u64,
    pub X17: u64,
    pub X18: u64,
    pub X19: u64,
    pub X20: u64,
    pub X21: u64,
    pub X22: u64,
    pub X23: u64,
    pub X24: u64,
    pub X25: u64,
    pub X26: u64,
    pub X27: u64,
    pub X28: u64,
    pub Fp: u64,
    pub Lr: u64,
}
pub type CONTEXT_FLAGS = u32;
pub const CREATE_ALWAYS: FILE_CREATION_DISPOSITION = 2;
pub const CREATE_NO_WINDOW: PROCESS_CREATION_FLAGS = 134217728;
pub const CREATE_SUSPENDED: PROCESS_CREATION_FLAGS = 4;
pub const CREATE_UNICODE_ENVIRONMENT: PROCESS_CREATION_FLAGS = 1024;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CRYPTPROTECT_PROMPTSTRUCT {
    pub cbSize: u32,
    pub dwPromptFlags: u32,
    pub hwndApp: HWND,
    pub szPrompt: PWSTR,
}
pub const CRYPTPROTECT_UI_FORBIDDEN: u32 = 1;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CRYPT_INTEGER_BLOB {
    pub cbData: u32,
    pub pbData: *mut u8,
}
pub const ComputerNameDnsHostname: COMPUTER_NAME_FORMAT = 1;
pub type DUPLICATE_HANDLE_OPTIONS = u32;
pub const DUPLICATE_SAME_ACCESS: DUPLICATE_HANDLE_OPTIONS = 2;
pub const ERROR_ACCESS_DENIED: WIN32_ERROR = 5;
pub const ERROR_ALREADY_EXISTS: WIN32_ERROR = 183;
pub const ERROR_FILE_NOT_FOUND: WIN32_ERROR = 2;
pub const ERROR_INSUFFICIENT_BUFFER: WIN32_ERROR = 122;
pub const ERROR_IO_INCOMPLETE: WIN32_ERROR = 996;
pub const ERROR_IO_PENDING: WIN32_ERROR = 997;
pub const ERROR_MORE_DATA: WIN32_ERROR = 234;
pub const ERROR_NO_MORE_ITEMS: WIN32_ERROR = 259;
pub const ERROR_PIPE_BUSY: WIN32_ERROR = 231;
pub const ERROR_PIPE_CONNECTED: WIN32_ERROR = 535;
pub const ERROR_SUCCESS: WIN32_ERROR = 0;
pub const EXCEPTION_ACCESS_VIOLATION: NTSTATUS = 0xC0000005_u32 as _;
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
pub const EXCEPTION_ILLEGAL_INSTRUCTION: NTSTATUS = 0xC000001D_u32 as _;
pub const EXCEPTION_INT_DIVIDE_BY_ZERO: NTSTATUS = 0xC0000094_u32 as _;
pub const EXCEPTION_IN_PAGE_ERROR: NTSTATUS = 0xC0000006_u32 as _;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EXCEPTION_POINTERS {
    pub ExceptionRecord: *mut EXCEPTION_RECORD,
    pub ContextRecord: *mut CONTEXT,
}
pub const EXCEPTION_PRIV_INSTRUCTION: NTSTATUS = 0xC0000096_u32 as _;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EXCEPTION_RECORD {
    pub ExceptionCode: NTSTATUS,
    pub ExceptionFlags: u32,
    pub ExceptionRecord: *mut Self,
    pub ExceptionAddress: *mut core::ffi::c_void,
    pub NumberParameters: u32,
    pub ExceptionInformation: [usize; 15],
}
impl Default for EXCEPTION_RECORD {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub const EXCEPTION_STACK_OVERFLOW: NTSTATUS = 0xC00000FD_u32 as _;
pub const EXTENDED_STARTUPINFO_PRESENT: PROCESS_CREATION_FLAGS = 524288;
pub type FARPROC = Option<unsafe extern "system" fn() -> isize>;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}
pub const FILE_ATTRIBUTE_NORMAL: FILE_FLAGS_AND_ATTRIBUTES = 128;
pub type FILE_CREATION_DISPOSITION = u32;
pub type FILE_FLAGS_AND_ATTRIBUTES = u32;
pub const FILE_FLAG_FIRST_PIPE_INSTANCE: FILE_FLAGS_AND_ATTRIBUTES = 524288;
pub const FILE_FLAG_OVERLAPPED: FILE_FLAGS_AND_ATTRIBUTES = 1073741824;
pub const FILE_NAME_NORMALIZED: GETFINALPATHNAMEBYHANDLE_FLAGS = 0;
pub const FILE_SHARE_DELETE: FILE_SHARE_MODE = 4;
pub type FILE_SHARE_MODE = u32;
pub const FILE_SHARE_READ: FILE_SHARE_MODE = 1;
pub const FILE_SHARE_WRITE: FILE_SHARE_MODE = 2;
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy)]
pub struct FLOATING_SAVE_AREA {
    pub ControlWord: u32,
    pub StatusWord: u32,
    pub TagWord: u32,
    pub ErrorOffset: u32,
    pub ErrorSelector: u32,
    pub DataOffset: u32,
    pub DataSelector: u32,
    pub RegisterArea: [u8; 80],
    pub Spare0: u32,
}
#[cfg(target_arch = "x86")]
impl Default for FLOATING_SAVE_AREA {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy)]
pub struct FLOATING_SAVE_AREA {
    pub ControlWord: u32,
    pub StatusWord: u32,
    pub TagWord: u32,
    pub ErrorOffset: u32,
    pub ErrorSelector: u32,
    pub DataOffset: u32,
    pub DataSelector: u32,
    pub RegisterArea: [u8; 80],
    pub Cr0NpxState: u32,
}
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
impl Default for FLOATING_SAVE_AREA {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub const FOLDERID_Documents: GUID = GUID {
    data1: 0xfdd39ad0,
    data2: 0x238f,
    data3: 0x46af,
    data4: [173, 180, 108, 133, 72, 3, 105, 199],
};
pub const FOLDERID_LocalAppData: GUID = GUID {
    data1: 0xf1b32785,
    data2: 0x6fba,
    data3: 0x4fcf,
    data4: [157, 85, 123, 142, 127, 21, 112, 145],
};
pub const FOLDERID_Profile: GUID = GUID {
    data1: 0x5e6c858f,
    data2: 0x0e22,
    data3: 0x4760,
    data4: [154, 254, 234, 51, 23, 182, 113, 115],
};
pub const FOLDERID_RoamingAppData: GUID = GUID {
    data1: 0x3eb685db,
    data2: 0x65f9,
    data3: 0x4cf6,
    data4: [160, 58, 227, 239, 101, 114, 159, 61],
};
pub const GAA_FLAG_INCLUDE_GATEWAYS: GET_ADAPTERS_ADDRESSES_FLAGS = 128;
pub const GAA_FLAG_SKIP_ANYCAST: GET_ADAPTERS_ADDRESSES_FLAGS = 2;
pub const GAA_FLAG_SKIP_DNS_SERVER: GET_ADAPTERS_ADDRESSES_FLAGS = 8;
pub const GAA_FLAG_SKIP_MULTICAST: GET_ADAPTERS_ADDRESSES_FLAGS = 4;
pub type GENERIC_ACCESS_RIGHTS = u32;
pub const GENERIC_READ: GENERIC_ACCESS_RIGHTS = 2147483648;
pub const GENERIC_WRITE: GENERIC_ACCESS_RIGHTS = 1073741824;
pub type GETFINALPATHNAMEBYHANDLE_FLAGS = u32;
pub type GET_ADAPTERS_ADDRESSES_FLAGS = u32;
pub type GLOBAL_ALLOC_FLAGS = u32;
pub const GMEM_MOVEABLE: GLOBAL_ALLOC_FLAGS = 2;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}
pub type HANDLE = *mut core::ffi::c_void;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HARDWAREINPUT {
    pub uMsg: u32,
    pub wParamL: u16,
    pub wParamH: u16,
}
pub type HDC = *mut core::ffi::c_void;
pub type HGLOBAL = *mut core::ffi::c_void;
pub type HHOOK = *mut core::ffi::c_void;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HIDP_CAPS {
    pub Usage: u16,
    pub UsagePage: u16,
    pub InputReportByteLength: u16,
    pub OutputReportByteLength: u16,
    pub FeatureReportByteLength: u16,
    pub Reserved: [u16; 17],
    pub NumberLinkCollectionNodes: u16,
    pub NumberInputButtonCaps: u16,
    pub NumberInputValueCaps: u16,
    pub NumberInputDataIndices: u16,
    pub NumberOutputButtonCaps: u16,
    pub NumberOutputValueCaps: u16,
    pub NumberOutputDataIndices: u16,
    pub NumberFeatureButtonCaps: u16,
    pub NumberFeatureValueCaps: u16,
    pub NumberFeatureDataIndices: u16,
}
impl Default for HIDP_CAPS {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub const HIDP_STATUS_SUCCESS: NTSTATUS = 0x110000_u32 as _;
pub type HINSTANCE = *mut core::ffi::c_void;
pub type HKEY = *mut core::ffi::c_void;
pub const HKEY_CURRENT_USER: HKEY = -2147483647 as _;
pub const HKEY_LOCAL_MACHINE: HKEY = -2147483646 as _;
pub type HLOCAL = *mut core::ffi::c_void;
pub type HMODULE = *mut core::ffi::c_void;
pub type HOOKPROC =
    Option<unsafe extern "system" fn(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT>;
pub type HRESULT = i32;
pub type HWND = *mut core::ffi::c_void;
pub type IF_OPER_STATUS = i32;
pub const INFINITE: u32 = 4294967295;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct INPUT {
    pub r#type: INPUT_TYPE,
    pub Anonymous: INPUT_0,
}
impl Default for INPUT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union INPUT_0 {
    pub mi: MOUSEINPUT,
    pub ki: KEYBDINPUT,
    pub hi: HARDWAREINPUT,
}
impl Default for INPUT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub const INPUT_KEYBOARD: INPUT_TYPE = 1;
pub type INPUT_TYPE = u32;
pub const INVALID_HANDLE_VALUE: HANDLE = -1 as _;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IO_COUNTERS {
    pub ReadOperationCount: u64,
    pub WriteOperationCount: u64,
    pub OtherOperationCount: u64,
    pub ReadTransferCount: u64,
    pub WriteTransferCount: u64,
    pub OtherTransferCount: u64,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_ADDRESSES_LH {
    pub Anonymous1: IP_ADAPTER_ADDRESSES_LH_0,
    pub Next: *mut Self,
    pub AdapterName: PSTR,
    pub FirstUnicastAddress: *mut IP_ADAPTER_UNICAST_ADDRESS_LH,
    pub FirstAnycastAddress: *mut IP_ADAPTER_ANYCAST_ADDRESS_XP,
    pub FirstMulticastAddress: *mut IP_ADAPTER_MULTICAST_ADDRESS_XP,
    pub FirstDnsServerAddress: *mut IP_ADAPTER_DNS_SERVER_ADDRESS_XP,
    pub DnsSuffix: PWSTR,
    pub Description: PWSTR,
    pub FriendlyName: PWSTR,
    pub PhysicalAddress: [u8; 8],
    pub PhysicalAddressLength: u32,
    pub Anonymous2: IP_ADAPTER_ADDRESSES_LH_1,
    pub Mtu: u32,
    pub IfType: u32,
    pub OperStatus: IF_OPER_STATUS,
    pub Ipv6IfIndex: u32,
    pub ZoneIndices: [u32; 16],
    pub FirstPrefix: *mut IP_ADAPTER_PREFIX_XP,
    pub TransmitLinkSpeed: u64,
    pub ReceiveLinkSpeed: u64,
    pub FirstWinsServerAddress: *mut IP_ADAPTER_WINS_SERVER_ADDRESS_LH,
    pub FirstGatewayAddress: *mut IP_ADAPTER_GATEWAY_ADDRESS_LH,
    pub Ipv4Metric: u32,
    pub Ipv6Metric: u32,
    pub Luid: NET_LUID_LH,
    pub Dhcpv4Server: SOCKET_ADDRESS,
    pub CompartmentId: NET_IF_COMPARTMENT_ID,
    pub NetworkGuid: GUID,
    pub ConnectionType: NET_IF_CONNECTION_TYPE,
    pub TunnelType: TUNNEL_TYPE,
    pub Dhcpv6Server: SOCKET_ADDRESS,
    pub Dhcpv6ClientDuid: [u8; 130],
    pub Dhcpv6ClientDuidLength: u32,
    pub Dhcpv6Iaid: u32,
    pub FirstDnsSuffix: *mut IP_ADAPTER_DNS_SUFFIX,
}
impl Default for IP_ADAPTER_ADDRESSES_LH {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_ADDRESSES_LH_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_ADDRESSES_LH_0_0,
}
impl Default for IP_ADAPTER_ADDRESSES_LH_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_ADDRESSES_LH_0_0 {
    pub Length: u32,
    pub IfIndex: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_ADDRESSES_LH_1 {
    pub Flags: u32,
    pub Anonymous: IP_ADAPTER_ADDRESSES_LH_1_0,
}
impl Default for IP_ADAPTER_ADDRESSES_LH_1 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_ADDRESSES_LH_1_0 {
    pub _bitfield: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_ANYCAST_ADDRESS_XP {
    pub Anonymous: IP_ADAPTER_ANYCAST_ADDRESS_XP_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
}
impl Default for IP_ADAPTER_ANYCAST_ADDRESS_XP {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_ANYCAST_ADDRESS_XP_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_ANYCAST_ADDRESS_XP_0_0,
}
impl Default for IP_ADAPTER_ANYCAST_ADDRESS_XP_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_ANYCAST_ADDRESS_XP_0_0 {
    pub Length: u32,
    pub Flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_DNS_SERVER_ADDRESS_XP {
    pub Anonymous: IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
}
impl Default for IP_ADAPTER_DNS_SERVER_ADDRESS_XP {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0_0,
}
impl Default for IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0_0 {
    pub Length: u32,
    pub Reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_DNS_SUFFIX {
    pub Next: *mut Self,
    pub String: [u16; 256],
}
impl Default for IP_ADAPTER_DNS_SUFFIX {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_GATEWAY_ADDRESS_LH {
    pub Anonymous: IP_ADAPTER_GATEWAY_ADDRESS_LH_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
}
impl Default for IP_ADAPTER_GATEWAY_ADDRESS_LH {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_GATEWAY_ADDRESS_LH_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_GATEWAY_ADDRESS_LH_0_0,
}
impl Default for IP_ADAPTER_GATEWAY_ADDRESS_LH_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_GATEWAY_ADDRESS_LH_0_0 {
    pub Length: u32,
    pub Reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_MULTICAST_ADDRESS_XP {
    pub Anonymous: IP_ADAPTER_MULTICAST_ADDRESS_XP_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
}
impl Default for IP_ADAPTER_MULTICAST_ADDRESS_XP {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_MULTICAST_ADDRESS_XP_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_MULTICAST_ADDRESS_XP_0_0,
}
impl Default for IP_ADAPTER_MULTICAST_ADDRESS_XP_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_MULTICAST_ADDRESS_XP_0_0 {
    pub Length: u32,
    pub Flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_PREFIX_XP {
    pub Anonymous: IP_ADAPTER_PREFIX_XP_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
    pub PrefixLength: u32,
}
impl Default for IP_ADAPTER_PREFIX_XP {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_PREFIX_XP_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_PREFIX_XP_0_0,
}
impl Default for IP_ADAPTER_PREFIX_XP_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_PREFIX_XP_0_0 {
    pub Length: u32,
    pub Flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_UNICAST_ADDRESS_LH {
    pub Anonymous: IP_ADAPTER_UNICAST_ADDRESS_LH_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
    pub PrefixOrigin: NL_PREFIX_ORIGIN,
    pub SuffixOrigin: NL_SUFFIX_ORIGIN,
    pub DadState: NL_DAD_STATE,
    pub ValidLifetime: u32,
    pub PreferredLifetime: u32,
    pub LeaseLifetime: u32,
    pub OnLinkPrefixLength: u8,
}
impl Default for IP_ADAPTER_UNICAST_ADDRESS_LH {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_UNICAST_ADDRESS_LH_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_UNICAST_ADDRESS_LH_0_0,
}
impl Default for IP_ADAPTER_UNICAST_ADDRESS_LH_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_UNICAST_ADDRESS_LH_0_0 {
    pub Length: u32,
    pub Flags: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IP_ADAPTER_WINS_SERVER_ADDRESS_LH {
    pub Anonymous: IP_ADAPTER_WINS_SERVER_ADDRESS_LH_0,
    pub Next: *mut Self,
    pub Address: SOCKET_ADDRESS,
}
impl Default for IP_ADAPTER_WINS_SERVER_ADDRESS_LH {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union IP_ADAPTER_WINS_SERVER_ADDRESS_LH_0 {
    pub Alignment: u64,
    pub Anonymous: IP_ADAPTER_WINS_SERVER_ADDRESS_LH_0_0,
}
impl Default for IP_ADAPTER_WINS_SERVER_ADDRESS_LH_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct IP_ADAPTER_WINS_SERVER_ADDRESS_LH_0_0 {
    pub Length: u32,
    pub Reserved: u32,
}
pub const IfOperStatusUp: IF_OPER_STATUS = 1;
pub type JOBOBJECTINFOCLASS = i32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    pub PerProcessUserTimeLimit: i64,
    pub PerJobUserTimeLimit: i64,
    pub LimitFlags: JOB_OBJECT_LIMIT,
    pub MinimumWorkingSetSize: usize,
    pub MaximumWorkingSetSize: usize,
    pub ActiveProcessLimit: u32,
    pub Affinity: usize,
    pub PriorityClass: u32,
    pub SchedulingClass: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    pub BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    pub IoInfo: IO_COUNTERS,
    pub ProcessMemoryLimit: usize,
    pub JobMemoryLimit: usize,
    pub PeakProcessMemoryUsed: usize,
    pub PeakJobMemoryUsed: usize,
}
pub type JOB_OBJECT_LIMIT = u32;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: JOB_OBJECT_LIMIT = 8192;
pub const JobObjectExtendedLimitInformation: JOBOBJECTINFOCLASS = 9;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KBDLLHOOKSTRUCT {
    pub vkCode: u32,
    pub scanCode: u32,
    pub flags: KBDLLHOOKSTRUCT_FLAGS,
    pub time: u32,
    pub dwExtraInfo: usize,
}
pub type KBDLLHOOKSTRUCT_FLAGS = u32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KEYBDINPUT {
    pub wVk: VIRTUAL_KEY,
    pub wScan: u16,
    pub dwFlags: KEYBD_EVENT_FLAGS,
    pub time: u32,
    pub dwExtraInfo: usize,
}
pub type KEYBD_EVENT_FLAGS = u32;
pub const KEYEVENTF_KEYUP: KEYBD_EVENT_FLAGS = 2;
pub const KEY_NOTIFY: REG_SAM_FLAGS = 16;
pub const KEY_READ: REG_SAM_FLAGS = 131097;
pub const KEY_SET_VALUE: REG_SAM_FLAGS = 2;
pub const KEY_WRITE: REG_SAM_FLAGS = 131078;
pub const KF_FLAG_DEFAULT: KNOWN_FOLDER_FLAG = 0;
pub type KNOWN_FOLDER_FLAG = i32;
pub const LLKHF_EXTENDED: KBDLLHOOKSTRUCT_FLAGS = 1;
pub type LOAD_LIBRARY_FLAGS = u32;
pub const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: LOAD_LIBRARY_FLAGS = 4096;
pub const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: LOAD_LIBRARY_FLAGS = 256;
pub const LOAD_LIBRARY_SEARCH_SYSTEM32: LOAD_LIBRARY_FLAGS = 2048;
pub type LPARAM = isize;
pub type LPPROC_THREAD_ATTRIBUTE_LIST = *mut core::ffi::c_void;
pub type LPTOP_LEVEL_EXCEPTION_FILTER =
    Option<unsafe extern "system" fn(exceptioninfo: *const EXCEPTION_POINTERS) -> i32>;
pub type LRESULT = isize;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct M128A {
    pub Low: u64,
    pub High: i64,
}
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_INFORMATION {
    pub CallbackRoutine: MINIDUMP_CALLBACK_ROUTINE,
    pub CallbackParam: *mut core::ffi::c_void,
}
#[repr(C, packed(4))]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_INFORMATION {
    pub CallbackRoutine: MINIDUMP_CALLBACK_ROUTINE,
    pub CallbackParam: *mut core::ffi::c_void,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct MINIDUMP_CALLBACK_INPUT {
    pub ProcessId: u32,
    pub ProcessHandle: HANDLE,
    pub CallbackType: u32,
    pub Anonymous: MINIDUMP_CALLBACK_INPUT_0,
}
impl Default for MINIDUMP_CALLBACK_INPUT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union MINIDUMP_CALLBACK_INPUT_0 {
    pub Status: HRESULT,
    pub Thread: MINIDUMP_THREAD_CALLBACK,
    pub ThreadEx: MINIDUMP_THREAD_EX_CALLBACK,
    pub Module: MINIDUMP_MODULE_CALLBACK,
    pub IncludeThread: MINIDUMP_INCLUDE_THREAD_CALLBACK,
    pub IncludeModule: MINIDUMP_INCLUDE_MODULE_CALLBACK,
    pub Io: MINIDUMP_IO_CALLBACK,
    pub ReadMemoryFailure: MINIDUMP_READ_MEMORY_FAILURE_CALLBACK,
    pub SecondaryFlags: u32,
    pub VmQuery: MINIDUMP_VM_QUERY_CALLBACK,
    pub VmPreRead: MINIDUMP_VM_PRE_READ_CALLBACK,
    pub VmPostRead: MINIDUMP_VM_POST_READ_CALLBACK,
}
impl Default for MINIDUMP_CALLBACK_INPUT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct MINIDUMP_CALLBACK_OUTPUT {
    pub Anonymous: MINIDUMP_CALLBACK_OUTPUT_0,
}
impl Default for MINIDUMP_CALLBACK_OUTPUT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union MINIDUMP_CALLBACK_OUTPUT_0 {
    pub ModuleWriteFlags: u32,
    pub ThreadWriteFlags: u32,
    pub SecondaryFlags: u32,
    pub Anonymous1: MINIDUMP_CALLBACK_OUTPUT_0_0,
    pub Anonymous2: MINIDUMP_CALLBACK_OUTPUT_0_1,
    pub Handle: HANDLE,
    pub Anonymous3: MINIDUMP_CALLBACK_OUTPUT_0_2,
    pub Anonymous4: MINIDUMP_CALLBACK_OUTPUT_0_3,
    pub Anonymous5: MINIDUMP_CALLBACK_OUTPUT_0_4,
    pub Status: HRESULT,
}
impl Default for MINIDUMP_CALLBACK_OUTPUT_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_OUTPUT_0_0 {
    pub MemoryBase: u64,
    pub MemorySize: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_OUTPUT_0_1 {
    pub CheckCancel: BOOL,
    pub Cancel: BOOL,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_OUTPUT_0_2 {
    pub VmRegion: MINIDUMP_MEMORY_INFO,
    pub Continue: BOOL,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_OUTPUT_0_3 {
    pub VmQueryStatus: HRESULT,
    pub VmQueryResult: MINIDUMP_MEMORY_INFO,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_CALLBACK_OUTPUT_0_4 {
    pub VmReadStatus: HRESULT,
    pub VmReadBytesCompleted: u32,
}
pub type MINIDUMP_CALLBACK_ROUTINE = Option<
    unsafe extern "system" fn(
        callbackparam: *mut core::ffi::c_void,
        callbackinput: *const MINIDUMP_CALLBACK_INPUT,
        callbackoutput: *mut MINIDUMP_CALLBACK_OUTPUT,
    ) -> BOOL,
>;
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_EXCEPTION_INFORMATION {
    pub ThreadId: u32,
    pub ExceptionPointers: *mut EXCEPTION_POINTERS,
    pub ClientPointers: BOOL,
}
#[repr(C, packed(4))]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_EXCEPTION_INFORMATION {
    pub ThreadId: u32,
    pub ExceptionPointers: *mut EXCEPTION_POINTERS,
    pub ClientPointers: BOOL,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_INCLUDE_MODULE_CALLBACK {
    pub BaseOfImage: u64,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_INCLUDE_THREAD_CALLBACK {
    pub ThreadId: u32,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_IO_CALLBACK {
    pub Handle: HANDLE,
    pub Offset: u64,
    pub Buffer: *mut core::ffi::c_void,
    pub BufferBytes: u32,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_MEMORY_INFO {
    pub BaseAddress: u64,
    pub AllocationBase: u64,
    pub AllocationProtect: u32,
    pub __alignment1: u32,
    pub RegionSize: u64,
    pub State: VIRTUAL_ALLOCATION_TYPE,
    pub Protect: u32,
    pub Type: u32,
    pub __alignment2: u32,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_MODULE_CALLBACK {
    pub FullPath: PWSTR,
    pub BaseOfImage: u64,
    pub SizeOfImage: u32,
    pub CheckSum: u32,
    pub TimeDateStamp: u32,
    pub VersionInfo: VS_FIXEDFILEINFO,
    pub CvRecord: *mut core::ffi::c_void,
    pub SizeOfCvRecord: u32,
    pub MiscRecord: *mut core::ffi::c_void,
    pub SizeOfMiscRecord: u32,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_READ_MEMORY_FAILURE_CALLBACK {
    pub Offset: u64,
    pub Bytes: u32,
    pub FailureStatus: HRESULT,
}
#[repr(C, packed(4))]
#[cfg(any(target_arch = "arm64ec", target_arch = "x86", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub struct MINIDUMP_THREAD_CALLBACK {
    pub ThreadId: u32,
    pub ThreadHandle: HANDLE,
    pub Context: CONTEXT,
    pub SizeOfContext: u32,
    pub StackBase: u64,
    pub StackEnd: u64,
}
#[cfg(any(target_arch = "arm64ec", target_arch = "x86", target_arch = "x86_64"))]
impl Default for MINIDUMP_THREAD_CALLBACK {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(4))]
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub struct MINIDUMP_THREAD_CALLBACK {
    pub ThreadId: u32,
    pub ThreadHandle: HANDLE,
    pub Pad: u32,
    pub Context: CONTEXT,
    pub SizeOfContext: u32,
    pub StackBase: u64,
    pub StackEnd: u64,
}
#[cfg(target_arch = "aarch64")]
impl Default for MINIDUMP_THREAD_CALLBACK {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(4))]
#[cfg(any(target_arch = "arm64ec", target_arch = "x86", target_arch = "x86_64"))]
#[derive(Clone, Copy)]
pub struct MINIDUMP_THREAD_EX_CALLBACK {
    pub ThreadId: u32,
    pub ThreadHandle: HANDLE,
    pub Context: CONTEXT,
    pub SizeOfContext: u32,
    pub StackBase: u64,
    pub StackEnd: u64,
    pub BackingStoreBase: u64,
    pub BackingStoreEnd: u64,
}
#[cfg(any(target_arch = "arm64ec", target_arch = "x86", target_arch = "x86_64"))]
impl Default for MINIDUMP_THREAD_EX_CALLBACK {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(4))]
#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub struct MINIDUMP_THREAD_EX_CALLBACK {
    pub ThreadId: u32,
    pub ThreadHandle: HANDLE,
    pub Pad: u32,
    pub Context: CONTEXT,
    pub SizeOfContext: u32,
    pub StackBase: u64,
    pub StackEnd: u64,
    pub BackingStoreBase: u64,
    pub BackingStoreEnd: u64,
}
#[cfg(target_arch = "aarch64")]
impl Default for MINIDUMP_THREAD_EX_CALLBACK {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub type MINIDUMP_TYPE = i32;
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_USER_STREAM {
    pub Type: u32,
    pub BufferSize: u32,
    pub Buffer: *mut core::ffi::c_void,
}
#[repr(C, packed(4))]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_USER_STREAM {
    pub Type: u32,
    pub BufferSize: u32,
    pub Buffer: *mut core::ffi::c_void,
}
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_USER_STREAM_INFORMATION {
    pub UserStreamCount: u32,
    pub UserStreamArray: *mut MINIDUMP_USER_STREAM,
}
#[repr(C, packed(4))]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_USER_STREAM_INFORMATION {
    pub UserStreamCount: u32,
    pub UserStreamArray: *mut MINIDUMP_USER_STREAM,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_VM_POST_READ_CALLBACK {
    pub Offset: u64,
    pub Buffer: *mut core::ffi::c_void,
    pub Size: u32,
    pub Completed: u32,
    pub Status: HRESULT,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_VM_PRE_READ_CALLBACK {
    pub Offset: u64,
    pub Buffer: *mut core::ffi::c_void,
    pub Size: u32,
}
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct MINIDUMP_VM_QUERY_CALLBACK {
    pub Offset: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MOUSEINPUT {
    pub dx: i32,
    pub dy: i32,
    pub mouseData: u32,
    pub dwFlags: MOUSE_EVENT_FLAGS,
    pub time: u32,
    pub dwExtraInfo: usize,
}
pub type MOUSE_EVENT_FLAGS = u32;
pub const MOVEFILE_REPLACE_EXISTING: MOVE_FILE_FLAGS = 1;
pub const MOVEFILE_WRITE_THROUGH: MOVE_FILE_FLAGS = 8;
pub type MOVE_FILE_FLAGS = u32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MSLLHOOKSTRUCT {
    pub pt: POINT,
    pub mouseData: u32,
    pub flags: u32,
    pub time: u32,
    pub dwExtraInfo: usize,
}
pub const MiniDumpWithThreadInfo: MINIDUMP_TYPE = 4096;
pub type NAMED_PIPE_MODE = u32;
pub type NET_IF_COMPARTMENT_ID = u32;
pub type NET_IF_CONNECTION_TYPE = i32;
#[repr(C)]
#[derive(Clone, Copy)]
pub union NET_LUID_LH {
    pub Value: u64,
    pub Info: NET_LUID_LH_0,
}
impl Default for NET_LUID_LH {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NET_LUID_LH_0 {
    pub _bitfield: u64,
}
pub type NL_DAD_STATE = i32;
pub type NL_PREFIX_ORIGIN = i32;
pub type NL_SUFFIX_ORIGIN = i32;
pub type NTSTATUS = i32;
pub const OPEN_EXISTING: FILE_CREATION_DISPOSITION = 3;
#[repr(C)]
#[derive(Clone, Copy)]
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
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OVERLAPPED {
    pub Internal: usize,
    pub InternalHigh: usize,
    pub Anonymous: OVERLAPPED_0,
    pub hEvent: HANDLE,
}
impl Default for OVERLAPPED {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union OVERLAPPED_0 {
    pub Anonymous: OVERLAPPED_0_0,
    pub Pointer: *mut core::ffi::c_void,
}
impl Default for OVERLAPPED_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OVERLAPPED_0_0 {
    pub Offset: u32,
    pub OffsetHigh: u32,
}
pub type PCSTR = *const u8;
pub type PCWSTR = *const u16;
pub type PEEK_MESSAGE_REMOVE_TYPE = u32;
pub type PHIDP_PREPARSED_DATA = isize;
pub const PIPE_ACCESS_DUPLEX: FILE_FLAGS_AND_ATTRIBUTES = 3;
pub const PIPE_READMODE_MESSAGE: NAMED_PIPE_MODE = 2;
pub const PIPE_REJECT_REMOTE_CLIENTS: NAMED_PIPE_MODE = 8;
pub const PIPE_TYPE_MESSAGE: NAMED_PIPE_MODE = 4;
pub const PIPE_UNLIMITED_INSTANCES: u32 = 255;
pub const PIPE_WAIT: NAMED_PIPE_MODE = 0;
pub const PM_NOREMOVE: PEEK_MESSAGE_REMOVE_TYPE = 0;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}
pub type PRINT_WINDOW_FLAGS = u32;
pub type PROCESS_ACCESS_RIGHTS = u32;
pub type PROCESS_CREATION_FLAGS = u32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PROCESS_INFORMATION {
    pub hProcess: HANDLE,
    pub hThread: HANDLE,
    pub dwProcessId: u32,
    pub dwThreadId: u32,
}
pub type PROCESS_NAME_FORMAT = u32;
pub const PROCESS_NAME_WIN32: PROCESS_NAME_FORMAT = 0;
pub const PROCESS_QUERY_LIMITED_INFORMATION: PROCESS_ACCESS_RIGHTS = 4096;
pub const PROCESS_SYNCHRONIZE: PROCESS_ACCESS_RIGHTS = 1048576;
pub const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: u32 = 131074;
pub type PSECURITY_DESCRIPTOR = *mut core::ffi::c_void;
pub type PSID = *mut core::ffi::c_void;
pub type PSTR = *mut u8;
pub type PVECTORED_EXCEPTION_HANDLER =
    Option<unsafe extern "system" fn(exceptioninfo: *mut EXCEPTION_POINTERS) -> i32>;
pub type PWSTR = *mut u16;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RAWINPUTDEVICELIST {
    pub hDevice: HANDLE,
    pub dwType: RID_DEVICE_INFO_TYPE,
}
pub type RAW_INPUT_DEVICE_INFO_COMMAND = u32;
pub type REG_CREATE_KEY_DISPOSITION = u32;
pub const REG_NOTIFY_CHANGE_LAST_SET: REG_NOTIFY_FILTER = 4;
pub const REG_NOTIFY_CHANGE_NAME: REG_NOTIFY_FILTER = 1;
pub type REG_NOTIFY_FILTER = u32;
pub const REG_NOTIFY_THREAD_AGNOSTIC: REG_NOTIFY_FILTER = 268435456;
pub type REG_OPEN_CREATE_OPTIONS = u32;
pub const REG_OPTION_NON_VOLATILE: REG_OPEN_CREATE_OPTIONS = 0;
pub type REG_ROUTINE_FLAGS = u32;
pub type REG_SAM_FLAGS = u32;
pub const REG_SZ: REG_VALUE_TYPE = 1;
pub type REG_VALUE_TYPE = u32;
pub const REPLACEFILE_IGNORE_ACL_ERRORS: REPLACE_FILE_FLAGS = 4;
pub const REPLACEFILE_IGNORE_MERGE_ERRORS: REPLACE_FILE_FLAGS = 2;
pub type REPLACE_FILE_FLAGS = u32;
pub const RIDI_DEVICEINFO: RAW_INPUT_DEVICE_INFO_COMMAND = 536870923;
pub const RIDI_DEVICENAME: RAW_INPUT_DEVICE_INFO_COMMAND = 536870919;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RID_DEVICE_INFO {
    pub cbSize: u32,
    pub dwType: RID_DEVICE_INFO_TYPE,
    pub Anonymous: RID_DEVICE_INFO_0,
}
impl Default for RID_DEVICE_INFO {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union RID_DEVICE_INFO_0 {
    pub mouse: RID_DEVICE_INFO_MOUSE,
    pub keyboard: RID_DEVICE_INFO_KEYBOARD,
    pub hid: RID_DEVICE_INFO_HID,
}
impl Default for RID_DEVICE_INFO_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RID_DEVICE_INFO_HID {
    pub dwVendorId: u32,
    pub dwProductId: u32,
    pub dwVersionNumber: u32,
    pub usUsagePage: u16,
    pub usUsage: u16,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RID_DEVICE_INFO_KEYBOARD {
    pub dwType: u32,
    pub dwSubType: u32,
    pub dwKeyboardMode: u32,
    pub dwNumberOfFunctionKeys: u32,
    pub dwNumberOfIndicators: u32,
    pub dwNumberOfKeysTotal: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RID_DEVICE_INFO_MOUSE {
    pub dwId: u32,
    pub dwNumberOfButtons: u32,
    pub dwSampleRate: u32,
    pub fHasHorizontalWheel: BOOL,
}
pub type RID_DEVICE_INFO_TYPE = u32;
pub const RIM_TYPEHID: RID_DEVICE_INFO_TYPE = 2;
pub const RRF_RT_REG_DWORD: REG_ROUTINE_FLAGS = 16;
pub const RRF_RT_REG_SZ: REG_ROUTINE_FLAGS = 2;
pub const SDDL_REVISION_1: u32 = 1;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: u32,
    pub lpSecurityDescriptor: *mut core::ffi::c_void,
    pub bInheritHandle: BOOL,
}
pub const SECURITY_IDENTIFICATION: FILE_FLAGS_AND_ATTRIBUTES = 65536;
pub const SECURITY_SQOS_PRESENT: FILE_FLAGS_AND_ATTRIBUTES = 1048576;
pub const SEE_MASK_FLAG_NO_UI: u32 = 1024;
pub const SEE_MASK_NOASYNC: u32 = 256;
pub const SHCNE_ASSOCCHANGED: SHCNE_ID = 134217728;
pub type SHCNE_ID = u32;
pub type SHCNF_FLAGS = u32;
pub const SHCNF_IDLIST: SHCNF_FLAGS = 0;
#[repr(C, packed(1))]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy)]
pub struct SHELLEXECUTEINFOW {
    pub cbSize: u32,
    pub fMask: u32,
    pub hwnd: HWND,
    pub lpVerb: PWSTR,
    pub lpFile: PWSTR,
    pub lpParameters: PWSTR,
    pub lpDirectory: PWSTR,
    pub nShow: i32,
    pub hInstApp: HINSTANCE,
    pub lpIDList: *mut core::ffi::c_void,
    pub lpClass: PWSTR,
    pub hkeyClass: HKEY,
    pub dwHotKey: u32,
    pub Anonymous: SHELLEXECUTEINFOW_0,
    pub hProcess: HANDLE,
}
#[cfg(target_arch = "x86")]
impl Default for SHELLEXECUTEINFOW {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C, packed(1))]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy)]
pub union SHELLEXECUTEINFOW_0 {
    pub hIcon: HANDLE,
    pub hMonitor: HANDLE,
}
#[cfg(target_arch = "x86")]
impl Default for SHELLEXECUTEINFOW_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy)]
pub struct SHELLEXECUTEINFOW {
    pub cbSize: u32,
    pub fMask: u32,
    pub hwnd: HWND,
    pub lpVerb: PWSTR,
    pub lpFile: PWSTR,
    pub lpParameters: PWSTR,
    pub lpDirectory: PWSTR,
    pub nShow: i32,
    pub hInstApp: HINSTANCE,
    pub lpIDList: *mut core::ffi::c_void,
    pub lpClass: PWSTR,
    pub hkeyClass: HKEY,
    pub dwHotKey: u32,
    pub Anonymous: SHELLEXECUTEINFOW_0,
    pub hProcess: HANDLE,
}
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
impl Default for SHELLEXECUTEINFOW {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy)]
pub union SHELLEXECUTEINFOW_0 {
    pub hIcon: HANDLE,
    pub hMonitor: HANDLE,
}
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
impl Default for SHELLEXECUTEINFOW_0 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub type SHOW_WINDOW_CMD = i32;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SID_AND_ATTRIBUTES {
    pub Sid: PSID,
    pub Attributes: u32,
}
pub const SM_SWAPBUTTON: SYSTEM_METRICS_INDEX = 23;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SOCKADDR {
    pub sa_family: ADDRESS_FAMILY,
    pub sa_data: [CHAR; 14],
}
impl Default for SOCKADDR {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SOCKET_ADDRESS {
    pub lpSockaddr: *mut SOCKADDR,
    pub iSockaddrLength: i32,
}
pub const STARTF_USESTDHANDLES: STARTUPINFOW_FLAGS = 256;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct STARTUPINFOEXW {
    pub StartupInfo: STARTUPINFOW,
    pub lpAttributeList: LPPROC_THREAD_ATTRIBUTE_LIST,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct STARTUPINFOW {
    pub cb: u32,
    pub lpReserved: PWSTR,
    pub lpDesktop: PWSTR,
    pub lpTitle: PWSTR,
    pub dwX: u32,
    pub dwY: u32,
    pub dwXSize: u32,
    pub dwYSize: u32,
    pub dwXCountChars: u32,
    pub dwYCountChars: u32,
    pub dwFillAttribute: u32,
    pub dwFlags: STARTUPINFOW_FLAGS,
    pub wShowWindow: u16,
    pub cbReserved2: u16,
    pub lpReserved2: *mut u8,
    pub hStdInput: HANDLE,
    pub hStdOutput: HANDLE,
    pub hStdError: HANDLE,
}
pub type STARTUPINFOW_FLAGS = u32;
pub const SW_RESTORE: SHOW_WINDOW_CMD = 9;
pub const SW_SHOW: SHOW_WINDOW_CMD = 5;
pub const SW_SHOWNORMAL: SHOW_WINDOW_CMD = 1;
pub type SYSTEM_METRICS_INDEX = i32;
pub type HSTRING = *mut core::ffi::c_void;
pub type TIMERPROC =
    Option<unsafe extern "system" fn(param0: HWND, param1: u32, param2: usize, param3: u32)>;
pub type TOKEN_ACCESS_MASK = u32;
pub type TOKEN_INFORMATION_CLASS = i32;
pub const TOKEN_QUERY: TOKEN_ACCESS_MASK = 8;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TOKEN_USER {
    pub User: SID_AND_ATTRIBUTES,
}
pub type TUNNEL_TYPE = i32;
pub const TokenUser: TOKEN_INFORMATION_CLASS = 1;
pub type VIRTUAL_ALLOCATION_TYPE = u32;
pub type VIRTUAL_KEY = u16;
pub const VK_F24: VIRTUAL_KEY = 135;
pub const VOLUME_NAME_DOS: GETFINALPATHNAMEBYHANDLE_FLAGS = 0;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VS_FIXEDFILEINFO {
    pub dwSignature: u32,
    pub dwStrucVersion: u32,
    pub dwFileVersionMS: u32,
    pub dwFileVersionLS: u32,
    pub dwProductVersionMS: u32,
    pub dwProductVersionLS: u32,
    pub dwFileFlagsMask: u32,
    pub dwFileFlags: VS_FIXEDFILEINFO_FILE_FLAGS,
    pub dwFileOS: VS_FIXEDFILEINFO_FILE_OS,
    pub dwFileType: u32,
    pub dwFileSubtype: u32,
    pub dwFileDateMS: u32,
    pub dwFileDateLS: u32,
}
pub type VS_FIXEDFILEINFO_FILE_FLAGS = u32;
pub type VS_FIXEDFILEINFO_FILE_OS = u32;
pub type WAIT_EVENT = u32;
pub const WAIT_OBJECT_0: WAIT_EVENT = 0;
pub const WAIT_TIMEOUT: WAIT_EVENT = 258;
pub const WH_KEYBOARD_LL: WINDOWS_HOOK_ID = 13;
pub const WH_MOUSE_LL: WINDOWS_HOOK_ID = 14;
pub type WIN32_ERROR = u32;
pub type WINDOWS_HOOK_ID = i32;
pub const WM_APP: u32 = 32768;
pub const WM_KEYDOWN: u32 = 256;
pub const WM_KEYUP: u32 = 257;
pub const WM_LBUTTONDOWN: u32 = 513;
pub const WM_LBUTTONUP: u32 = 514;
pub const WM_MBUTTONDOWN: u32 = 519;
pub const WM_MBUTTONUP: u32 = 520;
pub const WM_QUIT: u32 = 18;
pub const WM_RBUTTONDOWN: u32 = 516;
pub const WM_RBUTTONUP: u32 = 517;
pub const WM_SYSKEYDOWN: u32 = 260;
pub const WM_SYSKEYUP: u32 = 261;
pub const WM_TIMER: u32 = 275;
pub const WM_XBUTTONDOWN: u32 = 523;
pub const WM_XBUTTONUP: u32 = 524;
pub type WPARAM = usize;
#[repr(C)]
#[cfg(target_arch = "x86")]
#[derive(Clone, Copy)]
pub struct XSAVE_FORMAT {
    pub ControlWord: u16,
    pub StatusWord: u16,
    pub TagWord: u8,
    pub Reserved1: u8,
    pub ErrorOpcode: u16,
    pub ErrorOffset: u32,
    pub ErrorSelector: u16,
    pub Reserved2: u16,
    pub DataOffset: u32,
    pub DataSelector: u16,
    pub Reserved3: u16,
    pub MxCsr: u32,
    pub MxCsr_Mask: u32,
    pub FloatRegisters: [M128A; 8],
    pub XmmRegisters: [M128A; 8],
    pub Reserved4: [u8; 224],
}
#[cfg(target_arch = "x86")]
impl Default for XSAVE_FORMAT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
#[repr(C)]
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy)]
pub struct XSAVE_FORMAT {
    pub ControlWord: u16,
    pub StatusWord: u16,
    pub TagWord: u8,
    pub Reserved1: u8,
    pub ErrorOpcode: u16,
    pub ErrorOffset: u32,
    pub ErrorSelector: u16,
    pub Reserved2: u16,
    pub DataOffset: u32,
    pub DataSelector: u16,
    pub Reserved3: u16,
    pub MxCsr: u32,
    pub MxCsr_Mask: u32,
    pub FloatRegisters: [M128A; 8],
    pub XmmRegisters: [M128A; 16],
    pub Reserved4: [u8; 96],
}
#[cfg(any(
    target_arch = "aarch64",
    target_arch = "arm64ec",
    target_arch = "x86_64"
))]
impl Default for XSAVE_FORMAT {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
