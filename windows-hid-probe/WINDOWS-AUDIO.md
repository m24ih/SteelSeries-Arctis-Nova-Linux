# Windows audio prototype

## Application routing integration test (2026-11-09)

`examples/silent-session.rs` starts a short-lived silent stream on the default
output. Two instances were discovered under the same executable path on Game.
The application routing probe set both processes' console and multimedia policies
to Chat, then restored their originally empty policies. Their existing CPAL streams
remained on Game despite the policy change. After applying Chat again and
restarting the test executable, its new session appeared on Chat and both policies
were still Chat; the test then restored default. This demonstrates persistence for
this executable and system, not seamless migration for every Windows audio client.
Other clients, process restart races and device reconnects still require testing.

The probe reports the stored policy separately from the immediately observed
active-session endpoint IDs. A missing session or an observation error is not
confirmation of a move. A later stream change may not appear in that snapshot.
Windows stream switching depends on how the client activates and handles its
audio stream; see Microsoft's [automatic stream routing](https://learn.microsoft.com/en-us/windows/win32/coreaudio/automatic-stream-routing)
and [implementation considerations](https://learn.microsoft.com/en-us/windows/win32/coreaudio/stream-routing-implementation-considerations).
Those documents describe default-device routing, not a guarantee that this
undocumented per-application policy moves every existing stream.

To create a test session (silent, 30 seconds by default):

```powershell
cargo run --manifest-path windows-hid-probe/Cargo.toml --example silent-session -- 30
```

The probe writes persistent per-application Windows settings. Record the original
policies and restore them after an experiment; resetting to `default` is correct
only when the original policies were empty.

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
