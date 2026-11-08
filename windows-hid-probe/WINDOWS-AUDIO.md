# Windows audio prototype

The HID prototype reads the wheel independently of Sonar. `endpoints` lists active
Windows render endpoints and their identifiers without changing settings:

```powershell
cargo run --manifest-path windows-hid-probe/Cargo.toml --bin endpoints
```

Sonar endpoints only exist while Sonar is enabled. Two independent Game and Chat
endpoints are still required, together with a working path from both endpoints to
the physical headset. Listing or controlling endpoint volume does not create that
path. `sonar-audio --apply` is a 30-second experiment specifically for Sonar,
not a standalone mixer.

## Driver and routing work remaining

- Select a redistributable, appropriately signed Windows virtual audio driver that
  exposes two independent render endpoints and permits their audio to be captured.
  Verify its actual licensing, distribution rights, signing, and support for two
  simultaneous endpoints before adopting it.
- Capture each endpoint with WASAPI loopback, mix the streams and render to the
  selected physical headset. Handle format negotiation, clock drift, buffering,
  disconnects, feedback loops and protected audio restrictions.
- Associate endpoints using persistent identifiers, reject ambiguous selections,
  and apply wheel values without changing unrelated output devices. Restore any
  settings that the application explicitly changes when appropriate.
- Test simultaneous independently routed audio, balance extremes, reconnects,
  startup and shutdown on an actual Windows system.

Microsoft SysVAD is a driver development example, not a ready audio cable: its
sample simulates loopback rather than mixing actual streams. Driver installation,
test signing and security settings must not be changed as a side effect of running
the endpoint discovery tool.

References: [Microsoft SysVAD sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/),
[WASAPI loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording).
