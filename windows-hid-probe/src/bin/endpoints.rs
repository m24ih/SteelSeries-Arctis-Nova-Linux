use std::error::Error;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

fn main() -> Result<(), Box<dyn Error>> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for index in 0..devices.GetCount()? {
            let device = devices.Item(index)?;
            let id_ptr = device.GetId()?;
            let id = id_ptr.to_string();
            CoTaskMemFree(Some(id_ptr.0.cast()));
            let properties = device.OpenPropertyStore(STGM_READ)?;
            let value = properties.GetValue(&PKEY_Device_FriendlyName)?;
            let name_ptr = PropVariantToStringAlloc(&value)?;
            let name = name_ptr.to_string();
            CoTaskMemFree(Some(name_ptr.0.cast()));
            println!("{}\n  {}", name?, id?);
        }
    }
    Ok(())
}
