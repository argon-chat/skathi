#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppCapability(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    AppCapability,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl AppCapability {
    pub fn RequestAccessAsync(
        &self,
    ) -> windows_core::Result<windows_future::IAsyncOperation<AppCapabilityAccessStatus>> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).RequestAccessAsync)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn CheckAccess(&self) -> windows_core::Result<AppCapabilityAccessStatus> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).CheckAccess)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub fn Create(capabilityname: &windows_core::HSTRING) -> windows_core::Result<Self> {
        Self::IAppCapabilityStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).Create)(
                windows_core::Interface::as_raw(this),
                core::mem::transmute_copy(capabilityname),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn IAppCapabilityStatics<R, F: FnOnce(&IAppCapabilityStatics) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<AppCapability, IAppCapabilityStatics> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for AppCapability {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IAppCapability>();
}
unsafe impl windows_core::Interface for AppCapability {
    type Vtable = <IAppCapability as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IAppCapability as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for AppCapability {
    const NAME: &'static str = "Windows.Security.Authorization.AppCapabilityAccess.AppCapability";
}
unsafe impl Send for AppCapability {}
unsafe impl Sync for AppCapability {}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AppCapabilityAccessStatus(pub i32);
impl AppCapabilityAccessStatus {
    pub const DeniedBySystem: Self = Self(0);
    pub const NotDeclaredByApp: Self = Self(1);
    pub const DeniedByUser: Self = Self(2);
    pub const UserPromptRequired: Self = Self(3);
    pub const Allowed: Self = Self(4);
}
impl windows_core::imp::TypeKind for AppCapabilityAccessStatus {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for AppCapabilityAccessStatus {
    const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"enum(Windows.Security.Authorization.AppCapabilityAccess.AppCapabilityAccessStatus;i4)",
    );
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Security.Authorization.AppCapabilityAccess.AppCapabilityAccessStatus",
    );
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasicProperties(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    BasicProperties,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl windows_core::RuntimeType for BasicProperties {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IBasicProperties>();
}
unsafe impl windows_core::Interface for BasicProperties {
    type Vtable = <IBasicProperties as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IBasicProperties as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for BasicProperties {
    const NAME: &'static str = "Windows.Storage.FileProperties.BasicProperties";
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataPackage(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DataPackage,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl DataPackage {
    pub fn new() -> windows_core::Result<Self> {
        Self::IActivationFactory(|f| f.ActivateInstance::<Self>())
    }
    fn IActivationFactory<
        R,
        F: FnOnce(&windows_core::imp::IGenericFactory) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            DataPackage,
            windows_core::imp::IGenericFactory,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    pub fn Properties(&self) -> windows_core::Result<DataPackagePropertySet> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Properties)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn SetText(&self, value: &windows_core::HSTRING) -> windows_core::Result<()> {
        unsafe {
            (windows_core::Interface::vtable(self).SetText)(
                windows_core::Interface::as_raw(self),
                core::mem::transmute_copy(value),
            )
            .ok()
        }
    }
    pub fn SetStorageItems<P0>(&self, value: P0, readonly: bool) -> windows_core::Result<()>
    where
        P0: windows_core::Param<windows_collections::IIterable<IStorageItem>>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).SetStorageItems)(
                windows_core::Interface::as_raw(self),
                value.param().abi(),
                readonly,
            )
            .ok()
        }
    }
    pub fn SetWebLink<P0>(&self, value: P0) -> windows_core::Result<()>
    where
        P0: windows_core::Param<Uri>,
    {
        let this = &windows_core::Interface::cast::<IDataPackage2>(self)?;
        unsafe {
            (windows_core::Interface::vtable(this).SetWebLink)(
                windows_core::Interface::as_raw(this),
                value.param().abi(),
            )
            .ok()
        }
    }
    pub fn ShareCompleted<F>(&self, handler: F) -> windows_core::Result<windows_core::EventRevoker>
    where
        F: Fn(windows_core::Ref<Self>, windows_core::Ref<ShareCompletedEventArgs>) + Send + 'static,
    {
        let this = &windows_core::Interface::cast::<IDataPackage3>(self)?;
        let handler = <TypedEventHandler<Self, ShareCompletedEventArgs>>::new(move |a0, a1| {
            handler(a0, a1);
            Ok(())
        });
        unsafe {
            let mut result__ = core::mem::zeroed();
            let token__ = (windows_core::Interface::vtable(this).ShareCompleted)(
                windows_core::Interface::as_raw(this),
                windows_core::Interface::as_raw(&handler),
                &mut result__,
            )
            .map(|| result__)?;
            Ok(windows_core::EventRevoker::new(
                this.clone(),
                token__,
                windows_core::Interface::vtable(this).RemoveShareCompleted,
            ))
        }
    }
    pub fn ShareCanceled<F>(&self, handler: F) -> windows_core::Result<windows_core::EventRevoker>
    where
        F: Fn(windows_core::Ref<Self>, windows_core::Ref<windows_core::IInspectable>)
            + Send
            + 'static,
    {
        let this = &windows_core::Interface::cast::<IDataPackage4>(self)?;
        let handler = <TypedEventHandler<Self, windows_core::IInspectable>>::new(move |a0, a1| {
            handler(a0, a1);
            Ok(())
        });
        unsafe {
            let mut result__ = core::mem::zeroed();
            let token__ = (windows_core::Interface::vtable(this).ShareCanceled)(
                windows_core::Interface::as_raw(this),
                windows_core::Interface::as_raw(&handler),
                &mut result__,
            )
            .map(|| result__)?;
            Ok(windows_core::EventRevoker::new(
                this.clone(),
                token__,
                windows_core::Interface::vtable(this).RemoveShareCanceled,
            ))
        }
    }
}
impl windows_core::RuntimeType for DataPackage {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDataPackage>();
}
unsafe impl windows_core::Interface for DataPackage {
    type Vtable = <IDataPackage as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDataPackage as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for DataPackage {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.DataPackage";
}
unsafe impl Send for DataPackage {}
unsafe impl Sync for DataPackage {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataPackagePropertySet(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DataPackagePropertySet,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl DataPackagePropertySet {
    pub fn SetTitle(&self, value: &windows_core::HSTRING) -> windows_core::Result<()> {
        unsafe {
            (windows_core::Interface::vtable(self).SetTitle)(
                windows_core::Interface::as_raw(self),
                core::mem::transmute_copy(value),
            )
            .ok()
        }
    }
}
impl windows_core::RuntimeType for DataPackagePropertySet {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDataPackagePropertySet>();
}
unsafe impl windows_core::Interface for DataPackagePropertySet {
    type Vtable = <IDataPackagePropertySet as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDataPackagePropertySet as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for DataPackagePropertySet {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.DataPackagePropertySet";
}
unsafe impl Send for DataPackagePropertySet {}
unsafe impl Sync for DataPackagePropertySet {}
impl IntoIterator for DataPackagePropertySet {
    type Item =
        windows_collections::IKeyValuePair<windows_core::HSTRING, windows_core::IInspectable>;
    type IntoIter = windows_collections::BufferedIterator<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIterator::into_iter(&self)
    }
}
impl IntoIterator for &DataPackagePropertySet {
    type Item =
        windows_collections::IKeyValuePair<windows_core::HSTRING, windows_core::IInspectable>;
    type IntoIter = windows_collections::BufferedIterator<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        windows_collections::BufferedIterator::new(self.First().unwrap())
    }
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataRequest(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DataRequest,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl DataRequest {
    pub fn Data(&self) -> windows_core::Result<DataPackage> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Data)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
    pub fn SetData<P0>(&self, value: P0) -> windows_core::Result<()>
    where
        P0: windows_core::Param<DataPackage>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).SetData)(
                windows_core::Interface::as_raw(self),
                value.param().abi(),
            )
            .ok()
        }
    }
}
impl windows_core::RuntimeType for DataRequest {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDataRequest>();
}
unsafe impl windows_core::Interface for DataRequest {
    type Vtable = <IDataRequest as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDataRequest as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for DataRequest {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.DataRequest";
}
unsafe impl Send for DataRequest {}
unsafe impl Sync for DataRequest {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataRequestedEventArgs(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DataRequestedEventArgs,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl DataRequestedEventArgs {
    pub fn Request(&self) -> windows_core::Result<DataRequest> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Request)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
impl windows_core::RuntimeType for DataRequestedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDataRequestedEventArgs>();
}
unsafe impl windows_core::Interface for DataRequestedEventArgs {
    type Vtable = <IDataRequestedEventArgs as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDataRequestedEventArgs as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for DataRequestedEventArgs {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.DataRequestedEventArgs";
}
unsafe impl Send for DataRequestedEventArgs {}
unsafe impl Sync for DataRequestedEventArgs {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataTransferManager(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DataTransferManager,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl DataTransferManager {
    pub fn DataRequested<F>(&self, handler: F) -> windows_core::Result<windows_core::EventRevoker>
    where
        F: Fn(windows_core::Ref<Self>, windows_core::Ref<DataRequestedEventArgs>) + Send + 'static,
    {
        let handler = <TypedEventHandler<Self, DataRequestedEventArgs>>::new(move |a0, a1| {
            handler(a0, a1);
            Ok(())
        });
        unsafe {
            let mut result__ = core::mem::zeroed();
            let token__ = (windows_core::Interface::vtable(self).DataRequested)(
                windows_core::Interface::as_raw(self),
                windows_core::Interface::as_raw(&handler),
                &mut result__,
            )
            .map(|| result__)?;
            Ok(windows_core::EventRevoker::new(
                self.clone(),
                token__,
                windows_core::Interface::vtable(self).RemoveDataRequested,
            ))
        }
    }
    fn IDataTransferManagerStatics<
        R,
        F: FnOnce(&IDataTransferManagerStatics) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            DataTransferManager,
            IDataTransferManagerStatics,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn IDataTransferManagerStatics2<
        R,
        F: FnOnce(&IDataTransferManagerStatics2) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            DataTransferManager,
            IDataTransferManagerStatics2,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn IDataTransferManagerStatics3<
        R,
        F: FnOnce(&IDataTransferManagerStatics3) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            DataTransferManager,
            IDataTransferManagerStatics3,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for DataTransferManager {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDataTransferManager>();
}
unsafe impl windows_core::Interface for DataTransferManager {
    type Vtable = <IDataTransferManager as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDataTransferManager as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for DataTransferManager {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.DataTransferManager";
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileAttributes(pub u32);
impl FileAttributes {
    pub const Normal: Self = Self(0);
    pub const ReadOnly: Self = Self(1);
    pub const Directory: Self = Self(16);
    pub const Archive: Self = Self(32);
    pub const Temporary: Self = Self(256);
    pub const LocallyIncomplete: Self = Self(512);
}
impl windows_core::imp::TypeKind for FileAttributes {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for FileAttributes {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"enum(Windows.Storage.FileAttributes;u4)");
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.FileAttributes");
}
impl FileAttributes {
    pub const fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}
impl core::ops::BitOr for FileAttributes {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
impl core::ops::BitAnd for FileAttributes {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}
impl core::ops::BitOrAssign for FileAttributes {
    fn bitor_assign(&mut self, other: Self) {
        self.0.bitor_assign(other.0);
    }
}
impl core::ops::BitAndAssign for FileAttributes {
    fn bitand_assign(&mut self, other: Self) {
        self.0.bitand_assign(other.0);
    }
}
impl core::ops::Not for FileAttributes {
    type Output = Self;
    fn not(self) -> Self {
        Self(self.0.not())
    }
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Gamepad(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    Gamepad,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl Gamepad {
    pub fn FromGameController<P0>(gamecontroller: P0) -> windows_core::Result<Self>
    where
        P0: windows_core::Param<IGameController>,
    {
        Self::IGamepadStatics2(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).FromGameController)(
                windows_core::Interface::as_raw(this),
                gamecontroller.param().abi(),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn IGamepadStatics<R, F: FnOnce(&IGamepadStatics) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<Gamepad, IGamepadStatics> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn IGamepadStatics2<R, F: FnOnce(&IGamepadStatics2) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<Gamepad, IGamepadStatics2> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for Gamepad {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IGamepad>();
}
unsafe impl windows_core::Interface for Gamepad {
    type Vtable = <IGamepad as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IGamepad as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for Gamepad {
    const NAME: &'static str = "Windows.Gaming.Input.Gamepad";
}
unsafe impl Send for Gamepad {}
unsafe impl Sync for Gamepad {}
windows_core::imp::define_interface!(
    IAppCapability,
    IAppCapability_Vtbl,
    0x4c49d915_8a2a_4295_9437_2df7c396aff4
);
impl windows_core::RuntimeType for IAppCapability {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Security.Authorization.AppCapabilityAccess.IAppCapability",
    );
}
#[repr(C)]
pub struct IAppCapability_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    CapabilityName: usize,
    User: usize,
    pub RequestAccessAsync: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub CheckAccess: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut AppCapabilityAccessStatus,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IAppCapabilityStatics,
    IAppCapabilityStatics_Vtbl,
    0x7c353e2a_46ee_44e5_af3d_6ad3fc49bd22
);
impl windows_core::RuntimeType for IAppCapabilityStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Security.Authorization.AppCapabilityAccess.IAppCapabilityStatics",
    );
}
#[repr(C)]
pub struct IAppCapabilityStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    RequestAccessForCapabilitiesAsync: usize,
    RequestAccessForCapabilitiesForUserAsync: usize,
    pub Create: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IBasicProperties,
    IBasicProperties_Vtbl,
    0xd05d55db_785e_4a66_be02_9beec58aea81
);
impl windows_core::RuntimeType for IBasicProperties {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Storage.FileProperties.IBasicProperties",
    );
}
#[repr(C)]
pub struct IBasicProperties_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IDataPackage,
    IDataPackage_Vtbl,
    0x61ebf5c7_efea_4346_9554_981d7e198ffe
);
impl windows_core::RuntimeType for IDataPackage {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataPackage",
    );
}
#[repr(C)]
pub struct IDataPackage_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    GetView: usize,
    pub Properties: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    RequestedOperation: usize,
    SetRequestedOperation: usize,
    OperationCompleted: usize,
    RemoveOperationCompleted: usize,
    Destroyed: usize,
    RemoveDestroyed: usize,
    SetData: usize,
    SetDataProvider: usize,
    pub SetText: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    SetUri: usize,
    SetHtmlFormat: usize,
    ResourceMap: usize,
    SetRtf: usize,
    SetBitmap: usize,
    SetStorageItemsReadOnly: usize,
    pub SetStorageItems: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        bool,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataPackage2,
    IDataPackage2_Vtbl,
    0x041c1fe9_2409_45e1_a538_4c53eeee04a7
);
impl windows_core::RuntimeType for IDataPackage2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataPackage2",
    );
}
#[repr(C)]
pub struct IDataPackage2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    SetApplicationLink: usize,
    pub SetWebLink: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataPackage3,
    IDataPackage3_Vtbl,
    0x88f31f5d_787b_4d32_965a_a9838105a056
);
impl windows_core::RuntimeType for IDataPackage3 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataPackage3",
    );
}
#[repr(C)]
pub struct IDataPackage3_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub ShareCompleted: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut i64,
    ) -> windows_core::HRESULT,
    pub RemoveShareCompleted:
        unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataPackage4,
    IDataPackage4_Vtbl,
    0x13a24ec8_9382_536f_852a_3045e1b29a3b
);
impl windows_core::RuntimeType for IDataPackage4 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataPackage4",
    );
}
#[repr(C)]
pub struct IDataPackage4_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub ShareCanceled: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut i64,
    ) -> windows_core::HRESULT,
    pub RemoveShareCanceled:
        unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataPackagePropertySet,
    IDataPackagePropertySet_Vtbl,
    0xcd1c93eb_4c4c_443a_a8d3_f5c241e91689
);
impl windows_core::RuntimeType for IDataPackagePropertySet {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataPackagePropertySet",
    );
}
#[repr(C)]
pub struct IDataPackagePropertySet_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    Title: usize,
    pub SetTitle: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataRequest,
    IDataRequest_Vtbl,
    0x4341ae3b_fc12_4e53_8c02_ac714c415a27
);
impl windows_core::RuntimeType for IDataRequest {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataRequest",
    );
}
#[repr(C)]
pub struct IDataRequest_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub Data: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub SetData: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataRequestedEventArgs,
    IDataRequestedEventArgs_Vtbl,
    0xcb8ba807_6ac5_43c9_8ac5_9ba232163182
);
impl windows_core::RuntimeType for IDataRequestedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataRequestedEventArgs",
    );
}
#[repr(C)]
pub struct IDataRequestedEventArgs_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub Request: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataTransferManager,
    IDataTransferManager_Vtbl,
    0xa5caee9b_8708_49d1_8d36_67d25a8da00c
);
impl windows_core::RuntimeType for IDataTransferManager {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataTransferManager",
    );
}
#[repr(C)]
pub struct IDataTransferManager_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub DataRequested: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut i64,
    ) -> windows_core::HRESULT,
    pub RemoveDataRequested:
        unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IDataTransferManagerStatics,
    IDataTransferManagerStatics_Vtbl,
    0xa9da01aa_e00e_4cfe_aa44_2dd932dca3d8
);
impl windows_core::RuntimeType for IDataTransferManagerStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataTransferManagerStatics",
    );
}
#[repr(C)]
pub struct IDataTransferManagerStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IDataTransferManagerStatics2,
    IDataTransferManagerStatics2_Vtbl,
    0xc54ec2ec_9f97_4d63_9868_395e271ad8f5
);
impl windows_core::RuntimeType for IDataTransferManagerStatics2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataTransferManagerStatics2",
    );
}
#[repr(C)]
pub struct IDataTransferManagerStatics2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IDataTransferManagerStatics3,
    IDataTransferManagerStatics3_Vtbl,
    0x05845473_6c82_4f5c_ac23_62e458361fac
);
impl windows_core::RuntimeType for IDataTransferManagerStatics3 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IDataTransferManagerStatics3",
    );
}
#[repr(C)]
pub struct IDataTransferManagerStatics3_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IGameController,
    IGameController_Vtbl,
    0x1baf6522_5f64_42c5_8267_b9fe2215bfbd
);
impl windows_core::RuntimeType for IGameController {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IGameController");
}
windows_core::imp::interface_hierarchy!(
    IGameController,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl windows_core::RuntimeName for IGameController {
    const NAME: &'static str = "Windows.Gaming.Input.IGameController";
}
#[repr(C)]
pub struct IGameController_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IGamepad,
    IGamepad_Vtbl,
    0xbc7bb43c_0a69_3903_9e9d_a50f86a45de5
);
impl windows_core::RuntimeType for IGamepad {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IGamepad");
}
#[repr(C)]
pub struct IGamepad_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IGamepadStatics,
    IGamepadStatics_Vtbl,
    0x8bbce529_d49c_39e9_9560_e47dde96b7c8
);
impl windows_core::RuntimeType for IGamepadStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IGamepadStatics");
}
#[repr(C)]
pub struct IGamepadStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IGamepadStatics2,
    IGamepadStatics2_Vtbl,
    0x42676dc5_0856_47c4_9213_b395504c3a3c
);
impl windows_core::RuntimeType for IGamepadStatics2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IGamepadStatics2");
}
#[repr(C)]
pub struct IGamepadStatics2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub FromGameController: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IRawGameController,
    IRawGameController_Vtbl,
    0x7cad6d91_a7e1_4f71_9a78_33e9c5dfea62
);
impl windows_core::RuntimeType for IRawGameController {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IRawGameController");
}
#[repr(C)]
pub struct IRawGameController_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    AxisCount: usize,
    ButtonCount: usize,
    ForceFeedbackMotors: usize,
    pub HardwareProductId:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16) -> windows_core::HRESULT,
    pub HardwareVendorId:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut u16) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IRawGameController2,
    IRawGameController2_Vtbl,
    0x43c0c035_bb73_4756_a787_3ed6bea617bd
);
impl windows_core::RuntimeType for IRawGameController2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Gaming.Input.IRawGameController2");
}
#[repr(C)]
pub struct IRawGameController2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    SimpleHapticsControllers: usize,
    pub NonRoamableId: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DisplayName: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IRawGameControllerStatics,
    IRawGameControllerStatics_Vtbl,
    0xeb8d0792_e95a_4b19_afc7_0a59f8bf759e
);
impl windows_core::RuntimeType for IRawGameControllerStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Gaming.Input.IRawGameControllerStatics",
    );
}
#[repr(C)]
pub struct IRawGameControllerStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    RawGameControllerAdded: usize,
    RemoveRawGameControllerAdded: usize,
    RawGameControllerRemoved: usize,
    RemoveRawGameControllerRemoved: usize,
    pub RawGameControllers: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IShareCompletedEventArgs,
    IShareCompletedEventArgs_Vtbl,
    0x4574c442_f913_4f60_9df7_cc4060ab1916
);
impl windows_core::RuntimeType for IShareCompletedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.ApplicationModel.DataTransfer.IShareCompletedEventArgs",
    );
}
#[repr(C)]
pub struct IShareCompletedEventArgs_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    ISpatialAudioDeviceConfiguration,
    ISpatialAudioDeviceConfiguration_Vtbl,
    0xee830034_61cf_5749_9da4_10f0fe028199
);
impl windows_core::RuntimeType for ISpatialAudioDeviceConfiguration {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Media.Audio.ISpatialAudioDeviceConfiguration",
    );
}
#[repr(C)]
pub struct ISpatialAudioDeviceConfiguration_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    DeviceId: usize,
    IsSpatialAudioSupported: usize,
    IsSpatialAudioFormatSupported: usize,
    pub ActiveSpatialAudioFormat: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ISpatialAudioDeviceConfigurationStatics,
    ISpatialAudioDeviceConfigurationStatics_Vtbl,
    0x3ec37f7b_936d_4e04_9728_2827d9f758c4
);
impl windows_core::RuntimeType for ISpatialAudioDeviceConfigurationStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Media.Audio.ISpatialAudioDeviceConfigurationStatics",
    );
}
#[repr(C)]
pub struct ISpatialAudioDeviceConfigurationStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub GetForDeviceId: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ISpatialAudioFormatSubtypeStatics,
    ISpatialAudioFormatSubtypeStatics_Vtbl,
    0xb3de8a47_83ee_4266_a945_bedf507afeed
);
impl windows_core::RuntimeType for ISpatialAudioFormatSubtypeStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Media.Audio.ISpatialAudioFormatSubtypeStatics",
    );
}
#[repr(C)]
pub struct ISpatialAudioFormatSubtypeStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub WindowsSonic: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DolbyAtmosForHeadphones: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DolbyAtmosForHomeTheater: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DolbyAtmosForSpeakers: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DTSHeadphoneX: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
    pub DTSXUltra: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    ISpatialAudioFormatSubtypeStatics2,
    ISpatialAudioFormatSubtypeStatics2_Vtbl,
    0x4565e6cb_d95b_5621_b6af_0e8849c57c80
);
impl windows_core::RuntimeType for ISpatialAudioFormatSubtypeStatics2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Windows.Media.Audio.ISpatialAudioFormatSubtypeStatics2",
    );
}
#[repr(C)]
pub struct ISpatialAudioFormatSubtypeStatics2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub DTSXForHomeTheater: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IStorageFile,
    IStorageFile_Vtbl,
    0xfa3f6186_4214_428c_a64c_14c9ac7315ea
);
impl windows_core::RuntimeType for IStorageFile {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.IStorageFile");
}
windows_core::imp::interface_hierarchy!(
    IStorageFile,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl IStorageFile {
    pub fn Name(&self) -> windows_core::Result<windows_core::HSTRING> {
        let this = &windows_core::Interface::cast::<IStorageItem>(self)?;
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).Name)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
}
impl windows_core::RuntimeName for IStorageFile {
    const NAME: &'static str = "Windows.Storage.IStorageFile";
}
#[repr(C)]
pub struct IStorageFile_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IStorageFileStatics,
    IStorageFileStatics_Vtbl,
    0x5984c710_daf2_43c8_8bb4_a4d3eacfd03f
);
impl windows_core::RuntimeType for IStorageFileStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.IStorageFileStatics");
}
#[repr(C)]
pub struct IStorageFileStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub GetFileFromPathAsync: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IStorageFileStatics2,
    IStorageFileStatics2_Vtbl,
    0x5c76a781_212e_4af9_8f04_740cae108974
);
impl windows_core::RuntimeType for IStorageFileStatics2 {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.IStorageFileStatics2");
}
#[repr(C)]
pub struct IStorageFileStatics2_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IStorageItem,
    IStorageItem_Vtbl,
    0x4207a996_ca2f_42f7_bde8_8b10457a7f30
);
impl windows_core::RuntimeType for IStorageItem {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.IStorageItem");
}
windows_core::imp::interface_hierarchy!(
    IStorageItem,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl IStorageItem {
    pub fn Name(&self) -> windows_core::Result<windows_core::HSTRING> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).Name)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
}
impl windows_core::RuntimeName for IStorageItem {
    const NAME: &'static str = "Windows.Storage.IStorageItem";
}
#[repr(C)]
pub struct IStorageItem_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    RenameAsyncOverloadDefaultOptions: usize,
    RenameAsync: usize,
    DeleteAsyncOverloadDefaultOptions: usize,
    DeleteAsync: usize,
    GetBasicPropertiesAsync: usize,
    pub Name: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IUriEscapeStatics,
    IUriEscapeStatics_Vtbl,
    0xc1d432ba_c824_4452_a7fd_512bc3bbe9a1
);
impl windows_core::RuntimeType for IUriEscapeStatics {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Foundation.IUriEscapeStatics");
}
#[repr(C)]
pub struct IUriEscapeStatics_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IUriRuntimeClass,
    IUriRuntimeClass_Vtbl,
    0x9e365e57_48b2_4160_956f_c7385120bbfc
);
impl windows_core::RuntimeType for IUriRuntimeClass {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Foundation.IUriRuntimeClass");
}
#[repr(C)]
pub struct IUriRuntimeClass_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IUriRuntimeClassFactory,
    IUriRuntimeClassFactory_Vtbl,
    0x44a9796f_723e_4fdf_a218_033e75b0c084
);
impl windows_core::RuntimeType for IUriRuntimeClassFactory {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Foundation.IUriRuntimeClassFactory");
}
#[repr(C)]
pub struct IUriRuntimeClassFactory_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub CreateUri: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NameCollisionOption(pub i32);
impl NameCollisionOption {
    pub const GenerateUniqueName: Self = Self(0);
    pub const ReplaceExisting: Self = Self(1);
    pub const FailIfExists: Self = Self(2);
}
impl windows_core::imp::TypeKind for NameCollisionOption {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for NameCollisionOption {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"enum(Windows.Storage.NameCollisionOption;i4)");
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.NameCollisionOption");
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawGameController(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    RawGameController,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl RawGameController {
    pub fn HardwareProductId(&self) -> windows_core::Result<u16> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).HardwareProductId)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub fn HardwareVendorId(&self) -> windows_core::Result<u16> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).HardwareVendorId)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub fn NonRoamableId(&self) -> windows_core::Result<windows_core::HSTRING> {
        let this = &windows_core::Interface::cast::<IRawGameController2>(self)?;
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).NonRoamableId)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
    pub fn DisplayName(&self) -> windows_core::Result<windows_core::HSTRING> {
        let this = &windows_core::Interface::cast::<IRawGameController2>(self)?;
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DisplayName)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
    pub fn RawGameControllers() -> windows_core::Result<windows_collections::IVectorView<Self>> {
        Self::IRawGameControllerStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).RawGameControllers)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn IRawGameControllerStatics<
        R,
        F: FnOnce(&IRawGameControllerStatics) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            RawGameController,
            IRawGameControllerStatics,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for RawGameController {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IRawGameController>();
}
unsafe impl windows_core::Interface for RawGameController {
    type Vtable = <IRawGameController as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IRawGameController as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for RawGameController {
    const NAME: &'static str = "Windows.Gaming.Input.RawGameController";
}
unsafe impl Send for RawGameController {}
unsafe impl Sync for RawGameController {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShareCompletedEventArgs(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    ShareCompletedEventArgs,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl windows_core::RuntimeType for ShareCompletedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IShareCompletedEventArgs>();
}
unsafe impl windows_core::Interface for ShareCompletedEventArgs {
    type Vtable = <IShareCompletedEventArgs as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IShareCompletedEventArgs as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for ShareCompletedEventArgs {
    const NAME: &'static str = "Windows.ApplicationModel.DataTransfer.ShareCompletedEventArgs";
}
unsafe impl Send for ShareCompletedEventArgs {}
unsafe impl Sync for ShareCompletedEventArgs {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpatialAudioDeviceConfiguration(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    SpatialAudioDeviceConfiguration,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl SpatialAudioDeviceConfiguration {
    pub fn ActiveSpatialAudioFormat(&self) -> windows_core::Result<windows_core::HSTRING> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ActiveSpatialAudioFormat)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
    pub fn GetForDeviceId(deviceid: &windows_core::HSTRING) -> windows_core::Result<Self> {
        Self::ISpatialAudioDeviceConfigurationStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).GetForDeviceId)(
                windows_core::Interface::as_raw(this),
                core::mem::transmute_copy(deviceid),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn ISpatialAudioDeviceConfigurationStatics<
        R,
        F: FnOnce(&ISpatialAudioDeviceConfigurationStatics) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            SpatialAudioDeviceConfiguration,
            ISpatialAudioDeviceConfigurationStatics,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for SpatialAudioDeviceConfiguration {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, ISpatialAudioDeviceConfiguration>();
}
unsafe impl windows_core::Interface for SpatialAudioDeviceConfiguration {
    type Vtable = <ISpatialAudioDeviceConfiguration as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID =
        <ISpatialAudioDeviceConfiguration as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for SpatialAudioDeviceConfiguration {
    const NAME: &'static str = "Windows.Media.Audio.SpatialAudioDeviceConfiguration";
}
unsafe impl Send for SpatialAudioDeviceConfiguration {}
unsafe impl Sync for SpatialAudioDeviceConfiguration {}
pub struct SpatialAudioFormatSubtype;
impl SpatialAudioFormatSubtype {
    pub fn WindowsSonic() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).WindowsSonic)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DolbyAtmosForHeadphones() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DolbyAtmosForHeadphones)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DolbyAtmosForHomeTheater() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DolbyAtmosForHomeTheater)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DolbyAtmosForSpeakers() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DolbyAtmosForSpeakers)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DTSHeadphoneX() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DTSHeadphoneX)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DTSXUltra() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DTSXUltra)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    pub fn DTSXForHomeTheater() -> windows_core::Result<windows_core::HSTRING> {
        Self::ISpatialAudioFormatSubtypeStatics2(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).DTSXForHomeTheater)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        })
    }
    fn ISpatialAudioFormatSubtypeStatics<
        R,
        F: FnOnce(&ISpatialAudioFormatSubtypeStatics) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            SpatialAudioFormatSubtype,
            ISpatialAudioFormatSubtypeStatics,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn ISpatialAudioFormatSubtypeStatics2<
        R,
        F: FnOnce(&ISpatialAudioFormatSubtypeStatics2) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            SpatialAudioFormatSubtype,
            ISpatialAudioFormatSubtypeStatics2,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeName for SpatialAudioFormatSubtype {
    const NAME: &'static str = "Windows.Media.Audio.SpatialAudioFormatSubtype";
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageDeleteOption(pub i32);
impl StorageDeleteOption {
    pub const Default: Self = Self(0);
    pub const PermanentDelete: Self = Self(1);
}
impl windows_core::imp::TypeKind for StorageDeleteOption {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for StorageDeleteOption {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"enum(Windows.Storage.StorageDeleteOption;i4)");
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.StorageDeleteOption");
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageFile(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    StorageFile,
    windows_core::IUnknown,
    windows_core::IInspectable,
    IStorageFile
);
windows_core::imp::required_hierarchy!(StorageFile, IStorageItem);
impl StorageFile {
    pub fn GetFileFromPathAsync(
        path: &windows_core::HSTRING,
    ) -> windows_core::Result<windows_future::IAsyncOperation<Self>> {
        Self::IStorageFileStatics(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).GetFileFromPathAsync)(
                windows_core::Interface::as_raw(this),
                core::mem::transmute_copy(path),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    pub fn Name(&self) -> windows_core::Result<windows_core::HSTRING> {
        let this = &windows_core::Interface::cast::<IStorageItem>(self)?;
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).Name)(
                windows_core::Interface::as_raw(this),
                &mut result__,
            )
            .map(|| core::mem::transmute(result__))
        }
    }
    fn IStorageFileStatics<R, F: FnOnce(&IStorageFileStatics) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<StorageFile, IStorageFileStatics> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn IStorageFileStatics2<R, F: FnOnce(&IStorageFileStatics2) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<StorageFile, IStorageFileStatics2> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for StorageFile {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IStorageFile>();
}
unsafe impl windows_core::Interface for StorageFile {
    type Vtable = <IStorageFile as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IStorageFile as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for StorageFile {
    const NAME: &'static str = "Windows.Storage.StorageFile";
}
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageItemTypes(pub u32);
impl StorageItemTypes {
    pub const None: Self = Self(0);
    pub const File: Self = Self(1);
    pub const Folder: Self = Self(2);
}
impl windows_core::imp::TypeKind for StorageItemTypes {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for StorageItemTypes {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"enum(Windows.Storage.StorageItemTypes;u4)");
    const NAME: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"Windows.Storage.StorageItemTypes");
}
impl StorageItemTypes {
    pub const fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}
impl core::ops::BitOr for StorageItemTypes {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
impl core::ops::BitAnd for StorageItemTypes {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}
impl core::ops::BitOrAssign for StorageItemTypes {
    fn bitor_assign(&mut self, other: Self) {
        self.0.bitor_assign(other.0);
    }
}
impl core::ops::BitAndAssign for StorageItemTypes {
    fn bitand_assign(&mut self, other: Self) {
        self.0.bitand_assign(other.0);
    }
}
impl core::ops::Not for StorageItemTypes {
    type Output = Self;
    fn not(self) -> Self {
        Self(self.0.not())
    }
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedEventHandler<TSender, TResult>(
    windows_core::IUnknown,
    core::marker::PhantomData<TSender>,
    core::marker::PhantomData<TResult>,
)
where
    TSender: windows_core::RuntimeType + 'static,
    TResult: windows_core::RuntimeType + 'static;
unsafe impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
    > windows_core::Interface for TypedEventHandler<TSender, TResult>
{
    type Vtable = TypedEventHandler_Vtbl<TSender, TResult>;
    const IID: windows_core::GUID =
        windows_core::GUID::from_signature(<Self as windows_core::RuntimeType>::SIGNATURE);
}
impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
    > windows_core::RuntimeType for TypedEventHandler<TSender, TResult>
{
    const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
        .push_slice(b"pinterface({9de1c534-6ae1-11e0-84e1-18a905bcc53f}")
        .push_slice(b";")
        .push_other(TSender::SIGNATURE)
        .push_slice(b";")
        .push_other(TResult::SIGNATURE)
        .push_slice(b")");
}
impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
    > TypedEventHandler<TSender, TResult>
{
    pub fn new<
        F: Fn(windows_core::Ref<TSender>, windows_core::Ref<TResult>) -> windows_core::Result<()>
            + Send
            + 'static,
    >(
        invoke: F,
    ) -> Self {
        let com = windows_core::imp::DelegateBox::<Self, F>::new(
            &TypedEventHandlerBox::<TSender, TResult, F>::VTABLE,
            invoke,
        );
        unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
    }
    pub fn Invoke<P0, P1>(&self, sender: P0, args: P1) -> windows_core::Result<()>
    where
        P0: windows_core::Param<TSender>,
        P1: windows_core::Param<TResult>,
    {
        unsafe {
            (windows_core::Interface::vtable(self).Invoke)(
                windows_core::Interface::as_raw(self),
                sender.param().abi(),
                args.param().abi(),
            )
            .ok()
        }
    }
}
#[repr(C)]
pub struct TypedEventHandler_Vtbl<TSender, TResult>
where
    TSender: windows_core::RuntimeType + 'static,
    TResult: windows_core::RuntimeType + 'static,
{
    base__: windows_core::IUnknown_Vtbl,
    Invoke: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        sender: windows_core::imp::AbiType<TSender>,
        args: windows_core::imp::AbiType<TResult>,
    ) -> windows_core::HRESULT,
    TSender: core::marker::PhantomData<TSender>,
    TResult: core::marker::PhantomData<TResult>,
}
struct TypedEventHandlerBox<
    TSender,
    TResult,
    F: Fn(windows_core::Ref<TSender>, windows_core::Ref<TResult>) -> windows_core::Result<()>
        + Send
        + 'static,
>(core::marker::PhantomData<(TSender, TResult, fn() -> F)>)
where
    TSender: windows_core::RuntimeType + 'static,
    TResult: windows_core::RuntimeType + 'static;
impl<
        TSender: windows_core::RuntimeType + 'static,
        TResult: windows_core::RuntimeType + 'static,
        F: Fn(windows_core::Ref<TSender>, windows_core::Ref<TResult>) -> windows_core::Result<()>
            + Send
            + 'static,
    > TypedEventHandlerBox<TSender, TResult, F>
{
    const VTABLE : TypedEventHandler_Vtbl < TSender , TResult , > = TypedEventHandler_Vtbl::< TSender , TResult , > { base__ : windows_core::IUnknown_Vtbl { QueryInterface : windows_core::imp::DelegateBox::< TypedEventHandler < TSender , TResult > , F >::QueryInterface , AddRef : windows_core::imp::DelegateBox::< TypedEventHandler < TSender , TResult > , F >::AddRef , Release : windows_core::imp::DelegateBox::< TypedEventHandler < TSender , TResult > , F >::Release , } , Invoke : Self::Invoke , TSender : core::marker::PhantomData::< TSender > , TResult : core::marker::PhantomData::< TResult > } ;
    unsafe extern "system" fn Invoke(
        this: *mut core::ffi::c_void,
        sender: windows_core::imp::AbiType<TSender>,
        args: windows_core::imp::AbiType<TResult>,
    ) -> windows_core::HRESULT {
        unsafe {
            let this = &mut *(this as *mut *mut core::ffi::c_void
                as *mut windows_core::imp::DelegateBox<TypedEventHandler<TSender, TResult>, F>);
            (this.invoke)(
                core::mem::transmute_copy(&sender),
                core::mem::transmute_copy(&args),
            )
            .into()
        }
    }
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Uri(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(Uri, windows_core::IUnknown, windows_core::IInspectable);
impl Uri {
    pub fn CreateUri(uri: &windows_core::HSTRING) -> windows_core::Result<Self> {
        Self::IUriRuntimeClassFactory(|this| unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(this).CreateUri)(
                windows_core::Interface::as_raw(this),
                core::mem::transmute_copy(uri),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        })
    }
    fn IUriEscapeStatics<R, F: FnOnce(&IUriEscapeStatics) -> windows_core::Result<R>>(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<Uri, IUriEscapeStatics> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
    fn IUriRuntimeClassFactory<
        R,
        F: FnOnce(&IUriRuntimeClassFactory) -> windows_core::Result<R>,
    >(
        callback: F,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<Uri, IUriRuntimeClassFactory> =
            windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}
impl windows_core::RuntimeType for Uri {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IUriRuntimeClass>();
}
unsafe impl windows_core::Interface for Uri {
    type Vtable = <IUriRuntimeClass as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IUriRuntimeClass as windows_core::Interface>::IID;
}
impl windows_core::RuntimeName for Uri {
    const NAME: &'static str = "Windows.Foundation.Uri";
}
unsafe impl Send for Uri {}
unsafe impl Sync for Uri {}
