use arctis_windows_hid_probe::routing::{route, PolicyStore};
use std::ffi::c_void;
use windows::core::{define_interface, IInspectable_Vtbl, Interface, HRESULT, HSTRING};
use windows::Win32::System::WinRT::{RoGetActivationFactory, RoInitialize, RO_INIT_MULTITHREADED};

// Undocumented Windows 10 21H2+ interface. Test mode restores the prior per-role policies.
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

unsafe fn get_policy(
    factory: &IAudioPolicyProbe,
    pid: u32,
    role: i32,
) -> windows::core::Result<HSTRING> {
    let mut value = std::ptr::null_mut();
    (factory.vtable().get)(factory.as_raw(), pid, 0, role, &mut value).ok()?;
    Ok(std::mem::transmute::<*mut c_void, HSTRING>(value))
}

unsafe fn set_policy(
    factory: &IAudioPolicyProbe,
    pid: u32,
    role: i32,
    value: &HSTRING,
) -> windows::core::Result<()> {
    let raw: *mut c_void = std::mem::transmute_copy(value);
    (factory.vtable().set)(factory.as_raw(), pid, 0, role, raw).ok()
}

struct WindowsPolicyStore<'a>(&'a IAudioPolicyProbe);

impl PolicyStore for WindowsPolicyStore<'_> {
    type Value = HSTRING;
    type Error = windows::core::Error;

    fn read(&mut self, pid: u32, role: i32) -> Result<HSTRING, Self::Error> {
        unsafe { get_policy(self.0, pid, role) }
    }

    fn write(&mut self, pid: u32, role: i32, value: &HSTRING) -> Result<(), Self::Error> {
        unsafe { set_policy(self.0, pid, role, value) }
    }
}

struct Restore<'a> {
    factory: &'a IAudioPolicyProbe,
    pid: u32,
    old: [HSTRING; 2],
    armed: bool,
}
impl Drop for Restore<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for role in 0..2 {
            if let Err(error) =
                unsafe { set_policy(self.factory, self.pid, role, &self.old[role as usize]) }
            {
                eprintln!("Failed to restore role {role}: {error}");
            } else {
                println!("Restored role {role}");
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
        let factory: IAudioPolicyProbe =
            RoGetActivationFactory(&HSTRING::from("Windows.Media.Internal.AudioPolicyConfig"))?;
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).is_some_and(|value| value == "--set") {
            let pids: Vec<u32> = args
                .get(2)
                .and_then(|value| {
                    value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<u32>, _>>()
                        .ok()
                })
                .filter(|values| !values.is_empty() && values.iter().all(|&value| value != 0))
                .ok_or_else(|| {
                    windows::core::Error::from_hresult(windows::core::HRESULT(0x80070057u32 as i32))
                })?;
            let endpoint = args
                .get(3)
                .filter(|value| value.starts_with("{0.0.0.") || value.as_str() == "default")
                .ok_or_else(|| {
                    windows::core::Error::from_hresult(windows::core::HRESULT(0x80070057u32 as i32))
                })?;
            let packed = if endpoint == "default" {
                HSTRING::new()
            } else {
                HSTRING::from(format!(
                    r"\\?\SWD#MMDEVAPI#{endpoint}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}"
                ))
            };
            route(&mut WindowsPolicyStore(&factory), &pids, &packed)?;
            println!("Application routing updated for PIDs {pids:?}.");
            return Ok(());
        }
        if let Some(pid) = args.get(1) {
            let pid: u32 = pid.parse().expect("PID must be an integer");
            let old = [get_policy(&factory, pid, 0)?, get_policy(&factory, pid, 1)?];
            println!(
                "Original policies: console={:?} multimedia={:?}",
                old[0].to_string_lossy(),
                old[1].to_string_lossy()
            );
            if let Some(endpoint) = args.get(2) {
                let restore = Restore {
                    factory: &factory,
                    pid,
                    old,
                    armed: true,
                };
                let packed = HSTRING::from(format!(
                    r"\\?\SWD#MMDEVAPI#{endpoint}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}"
                ));
                for role in 0..2 {
                    set_policy(&factory, pid, role, &packed)?;
                }
                println!("Test policy applied for 10 seconds");
                for role in 0..2 {
                    println!(
                        "role={role}: {}",
                        get_policy(&factory, pid, role)?.to_string_lossy()
                    );
                }
                std::thread::sleep(std::time::Duration::from_secs(10));
                drop(restore);
                for role in 0..2 {
                    println!(
                        "After restore role={role}: {}",
                        get_policy(&factory, pid, role)?.to_string_lossy()
                    );
                }
            }
        }
    }
    println!("Audio policy probe finished.");
    Ok(())
}
