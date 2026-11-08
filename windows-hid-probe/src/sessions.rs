//! Discover active render sessions while retaining process identity handles.

use std::collections::{BTreeMap, BTreeSet};
use windows::core::{Interface, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

pub struct Process {
    pub pid: u32,
    pub path: String,
    handle: HANDLE,
}

impl Process {
    pub fn open(pid: u32) -> windows::core::Result<Self> {
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )?;
            let mut process = Self {
                pid,
                path: String::new(),
                handle,
            };
            let mut buffer = [0u16; 32768];
            let mut len = buffer.len() as u32;
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )?;
            process.path = String::from_utf16_lossy(&buffer[..len as usize]);
            process.ensure_alive()?;
            Ok(process)
        }
    }

    pub fn ensure_alive(&self) -> windows::core::Result<()> {
        unsafe {
            match WaitForSingleObject(self.handle, 0) {
                WAIT_TIMEOUT => Ok(()),
                WAIT_OBJECT_0 => Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070490u32 as i32,
                ))),
                _ => Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80004005u32 as i32,
                ))),
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub struct Sessions {
    pub processes: BTreeMap<u32, Process>,
    pub groups: BTreeMap<String, BTreeMap<String, BTreeSet<u32>>>,
    pub inaccessible_pids: BTreeSet<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_identity_is_available_and_alive() {
        let process = Process::open(std::process::id()).unwrap();
        assert!(!process.path.is_empty());
        process.ensure_alive().unwrap();
    }

    #[test]
    fn nonexistent_process_is_rejected() {
        assert!(Process::open(0).is_err());
    }

    #[test]
    fn process_exiting_with_still_active_code_is_detected() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 2 127.0.0.1 >nul & exit /b 259"])
            .spawn()
            .unwrap();
        let process = Process::open(child.id()).unwrap();
        assert!(process.ensure_alive().is_ok());
        child.wait().unwrap();
        assert!(process.ensure_alive().is_err());
    }
}

/// Return whether an exact active render endpoint ID exists. COM must be initialized.
pub fn active_render_endpoint_exists(id: &str) -> windows::core::Result<bool> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let endpoints = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for index in 0..endpoints.GetCount()? {
            let ptr = endpoints.Item(index)?.GetId()?;
            let value = ptr.to_string();
            CoTaskMemFree(Some(ptr.0.cast()));
            if value?.eq_ignore_ascii_case(id) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// COM must already be initialized on the calling thread.
pub fn active_sessions() -> windows::core::Result<Sessions> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let endpoints = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut result = Sessions {
            processes: BTreeMap::new(),
            groups: BTreeMap::new(),
            inaccessible_pids: BTreeSet::new(),
        };
        for index in 0..endpoints.GetCount()? {
            let endpoint = endpoints.Item(index)?;
            let props = endpoint.OpenPropertyStore(STGM_READ)?;
            let value = props.GetValue(&PKEY_Device_FriendlyName)?;
            let ptr = PropVariantToStringAlloc(&value)?;
            let name = ptr.to_string();
            CoTaskMemFree(Some(ptr.0.cast()));
            let name = name?;
            let manager: IAudioSessionManager2 = endpoint.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for n in 0..sessions.GetCount()? {
                let session = sessions.GetSession(n)?;
                if session.GetState()? != AudioSessionStateActive {
                    continue;
                }
                let control: IAudioSessionControl2 = session.cast()?;
                let pid = control.GetProcessId()?;
                if pid == 0 || pid == std::process::id() {
                    continue;
                }
                if !result.processes.contains_key(&pid) {
                    match Process::open(pid) {
                        Ok(process) => {
                            result.processes.insert(pid, process);
                        }
                        Err(_) => {
                            result.inaccessible_pids.insert(pid);
                            continue;
                        }
                    }
                }
                let path = &result.processes[&pid].path;
                if path
                    .rsplit('\\')
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case("virtual-outputs.exe"))
                {
                    continue;
                }
                result
                    .groups
                    .entry(path.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default()
                    .insert(pid);
            }
        }
        Ok(result)
    }
}
