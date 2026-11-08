use arctis_windows_hid_probe::sessions::active_sessions;
use std::error::Error;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

fn main() -> Result<(), Box<dyn Error>> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let sessions = active_sessions()?;
    for pid in &sessions.inaccessible_pids {
        eprintln!("Cannot inspect process {pid}; skipping");
    }
    for (path, outputs) in &sessions.groups {
        println!("{path}");
        for (output, pids) in outputs {
            println!("  {}: {pids:?}\n    {}", output.name, output.id);
        }
    }
    Ok(())
}
