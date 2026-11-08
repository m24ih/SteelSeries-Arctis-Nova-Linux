use std::ffi::c_void;
use windows::core::{define_interface, IInspectable_Vtbl, Interface, HRESULT, HSTRING};
use windows::Win32::System::WinRT::{RoGetActivationFactory, RoInitialize, RO_INIT_MULTITHREADED};

// Undocumented Windows 10 21H2+ interface; probe activation only, do not mutate routing.
// Interface IID and method ordering: https://github.com/File-New-Project/EarTrumpet/blob/master/EarTrumpet/Interop/MMDeviceAPI/IAudioPolicyConfigFactoryVariantFor21H2.cs
define_interface!(
    IAudioPolicyProbe,
    IAudioPolicyProbe_Vtbl,
    0xab3d4648_e242_459f_b02f_541c70306324
);
#[repr(C)]
pub struct IAudioPolicyProbe_Vtbl {
    pub base__: IInspectable_Vtbl,
    pub reserved: [usize; 19],
    pub set: unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> HRESULT,
    pub get: unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut *mut c_void) -> HRESULT,
}

fn main() -> windows::core::Result<()> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
        let factory: IAudioPolicyProbe =
            RoGetActivationFactory(&HSTRING::from("Windows.Media.Internal.AudioPolicyConfig"))?;
        if let Some(pid) = std::env::args().nth(1) {
            let pid: u32 = pid.parse().expect("PID must be an integer");
            for role in 0..=1 {
                let mut value = std::ptr::null_mut();
                let result = (factory.vtable().get)(factory.as_raw(), pid, 0, role, &mut value);
                if result.is_ok() {
                    let hstring: HSTRING = std::mem::transmute(value);
                    println!("role={role}: {}", hstring.to_string_lossy());
                } else {
                    println!("role={role}: {result:?}");
                }
            }
        }
    }
    println!("Windows 21H2+ audio policy interface activated; no routing changed.");
    Ok(())
}
