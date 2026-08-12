use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use std::sync::atomic::{AtomicI32, Ordering};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1, D3D_FEATURE_LEVEL_9_2,
    D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::Interface;

#[derive(thiserror::Error, Eq, PartialEq, Clone, Debug)]
pub enum Error {
    #[error("Failed to create DirectX device with the recommended feature levels")]
    FeatureLevelNotSatisfied,
    #[error("Windows API Error: {0}")]
    WindowsError(#[from] windows::core::Error),
}

/// A wrapper to send a DirectX device across threads.
pub struct SendDirectX<T>(pub T);

impl<T> SendDirectX<T> {
    /// Creates a new `SendDirectX` instance.
    ///
    /// # Arguments
    ///
    /// * `device` - The DirectX device.
    ///
    /// # Returns
    ///
    /// Returns a new `SendDirectX` instance.
    #[must_use]
    #[inline]
    pub const fn new(device: T) -> Self {
        Self(device)
    }
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for SendDirectX<T> {}

/// Номер адаптера, на котором создавать устройство. -1 — как раньше, выбор
/// оставлен DXGI.
///
/// Статик, а не параметр: путь до `create_d3d_device` идёт через
/// `start_free_threaded` и `Settings`, и протаскивать адаптер через всю
/// цепочку ради одной настройки дороже, чем оно того стоит.
static PREFERRED_ADAPTER: AtomicI32 = AtomicI32::new(-1);

/// Задаёт адаптер для последующих захватов.
///
/// Нужно там, где `D3D11CreateDevice` на адаптере по умолчанию рушит процесс
/// целиком: дефект драйвера портит кучу, перехватить это нельзя, и остаётся
/// только не ходить туда второй раз.
#[inline]
pub fn set_preferred_adapter(index: Option<u32>) {
    PREFERRED_ADAPTER.store(index.map_or(-1, |i| i as i32), Ordering::Relaxed);
}

/// Creates an `ID3D11Device` and an `ID3D11DeviceContext`.
#[inline]
pub fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext), Error> {
    // Array of Direct3D feature levels.
    // The feature levels are listed in descending order of capability.
    // The highest feature level supported by the system is at index 0.
    // The lowest feature level supported by the system is at the last index.
    // Только 11_1 и 11_0. Всё, что ниже, ниже по коду всё равно отвергается
    // проверкой feature_level < 11_0 — то есть пять уровней из семи
    // запрашивались впустую, но заставляли драйвер пройти по своим путям
    // совместимости. На драйвере NVIDIA 32.0.16.1062 D3D11CreateDevice в этом
    // месте рушил кучу целиком (#66), поэтому лишнее убрано.
    let feature_flags = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];

    let mut d3d_device = None;
    let mut feature_level = D3D_FEATURE_LEVEL::default();
    let mut d3d_device_context = None;

    // При явно заданном адаптере тип драйвера ОБЯЗАН быть UNKNOWN — этого
    // требует сам API, иначе вызов вернёт ошибку.
    let chosen = PREFERRED_ADAPTER.load(Ordering::Relaxed);
    let adapter: Option<IDXGIAdapter> = if chosen >= 0 {
        unsafe {
            CreateDXGIFactory1::<IDXGIFactory1>()
                .ok()
                .and_then(|f| f.EnumAdapters1(chosen as u32).ok())
                .map(Into::into)
        }
    } else {
        None
    };
    let driver_type = if adapter.is_some() {
        D3D_DRIVER_TYPE_UNKNOWN
    } else {
        D3D_DRIVER_TYPE_HARDWARE
    };
    log::info!("wc: создаём устройство D3D11, адаптер {chosen} (-1 = по умолчанию)");

    unsafe {
        D3D11CreateDevice(
            adapter.as_ref(),
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_flags),
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            Some(&mut feature_level),
            Some(&mut d3d_device_context),
        )?;
    };

    log::info!("wc: D3D11CreateDevice вернулся, уровень {:#x}", feature_level.0);
    if feature_level.0 < D3D_FEATURE_LEVEL_11_0.0 {
        return Err(Error::FeatureLevelNotSatisfied);
    }

    Ok((d3d_device.unwrap(), d3d_device_context.unwrap()))
}

/// Creates an `IDirect3DDevice` from an `ID3D11Device`.
#[inline]
pub fn create_direct3d_device(d3d_device: &ID3D11Device) -> Result<IDirect3DDevice, Error> {
    let dxgi_device: IDXGIDevice = d3d_device.cast()?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)? };
    let device: IDirect3DDevice = inspectable.cast()?;

    Ok(device)
}
