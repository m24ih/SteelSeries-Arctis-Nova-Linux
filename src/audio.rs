use crate::device::{CHAT_SINK_NAME, GAME_SINK_NAME, MAX_VOLUME_PERCENT};
use anyhow::{Context, Result};
use log::debug;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::{Command, Stdio};

const STEREO_CHANNELS: [&str; 2] = ["FL", "FR"];

/// Queries PulseAudio/PipeWire for the current default sink name.
pub(crate) fn get_default_sink() -> Result<String> {
    let output = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .context("Failed to execute pactl get-default-sink")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pactl get-default-sink failed: {}", stderr.trim());
    }

    let sink = String::from_utf8(output.stdout)?.trim().to_string();
    if sink.is_empty() {
        anyhow::bail!("Empty default sink");
    }
    Ok(sink)
}

/// Sets the system default audio sink via `pactl set-default-sink`.
pub(crate) fn set_default_sink(sink_name: &str) -> Result<()> {
    let status = Command::new("pactl")
        .args(["set-default-sink", sink_name])
        .status()
        .with_context(|| format!("Failed to set default sink to '{}'", sink_name))?;

    if !status.success() {
        anyhow::bail!("pactl set-default-sink exited with status {}", status);
    }
    Ok(())
}

/// Parses `pactl list short sinks` output to find the physical Arctis Nova 7 sink.
/// Inspects only the sink name column (column 1) so numeric sink indices (e.g. `57`)
/// or sample specs cannot cause false positives, and ignores our own virtual sinks.
/// Prefers entries whose sink name contains `"usb"`, `"playback"`, `"dot"`, or `"wireless"`.
pub(crate) fn parse_arctis_sink(sinks_output: &str) -> Option<String> {
    let mut fallback: Option<String> = None;

    for line in sinks_output.lines() {
        let Some(raw_name) = line.split('\t').nth(1) else {
            continue;
        };
        let name = raw_name.trim();
        if name.is_empty()
            || name.eq_ignore_ascii_case(GAME_SINK_NAME)
            || name.eq_ignore_ascii_case(CHAT_SINK_NAME)
        {
            continue;
        }

        let lower_name = name.to_lowercase();
        // Match on "nova" or "7" in addition to "arctis" within the sink name itself
        if lower_name.contains("arctis")
            && (lower_name.contains("7") || lower_name.contains("nova"))
        {
            if lower_name.contains("usb")
                || lower_name.contains("playback")
                || lower_name.contains("dot")
                || lower_name.contains("wireless")
            {
                return Some(name.to_string());
            }
            if fallback.is_none() {
                fallback = Some(name.to_string());
            }
        }
    }

    fallback
}

/// Finds the physical Arctis Nova 7 sink name in PipeWire/PulseAudio.
pub(crate) fn find_arctis_sink() -> Result<String> {
    let output = Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
        .context("Failed to list sinks")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pactl list short sinks failed: {}", stderr.trim());
    }

    let sinks = String::from_utf8(output.stdout)?;
    parse_arctis_sink(&sinks)
        .ok_or_else(|| anyhow::anyhow!("No Arctis Nova 7 device found in pactl output"))
}

/// Formats the PipeWire node properties string for a stereo virtual null-audio-sink.
pub(crate) fn format_null_sink_args(node_name: &str, description: &str) -> String {
    format!(
        r#"{{factory.name=support.null-audio-sink node.name={} node.description="{}" media.class=Audio/Sink monitor.channel-volumes=true object.linger=true audio.position=[FL FR]}}"#,
        node_name, description
    )
}

/// Creates a PipeWire virtual null-audio-sink with the given node name and description.
pub(crate) fn create_virtual_sink(node_name: &str, description: &str) -> Result<()> {
    let props = format_null_sink_args(node_name, description);
    let status = Command::new("pw-cli")
        .args(["create-node", "adapter", &props])
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("Failed to execute pw-cli create-node for {}", node_name))?;

    if !status.success() {
        anyhow::bail!("Failed to create {} sink", node_name);
    }
    Ok(())
}

/// Destroys a PipeWire node by name, suppressing stdout/stderr.
pub(crate) fn destroy_virtual_sink(node_name: &str) {
    let _ = Command::new("pw-cli")
        .args(["destroy", node_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Destroys both `Arctis_Game` and `Arctis_Chat` virtual sinks.
pub(crate) fn destroy_virtual_sinks() {
    destroy_virtual_sink(GAME_SINK_NAME);
    destroy_virtual_sink(CHAT_SINK_NAME);
}

/// Returns `true` if `pw-link` stderr indicates the link already exists.
pub(crate) fn is_link_exists_stderr(stderr: &str) -> bool {
    stderr.to_ascii_lowercase().contains("exists")
}

/// Idempotently links a virtual sink's `FL`/`FR` monitor ports to the physical device's playback ports.
pub(crate) fn link_sink_to_device(sink_name: &str, device_name: &str) -> Result<()> {
    for channel in STEREO_CHANNELS {
        let src = format!("{}:monitor_{}", sink_name, channel);
        let dst = format!("{}:playback_{}", device_name, channel);

        let output = Command::new("pw-link")
            .arg(&src)
            .arg(&dst)
            .output()
            .with_context(|| format!("Failed to execute pw-link for {}", channel))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If the link already exists, treat it as success rather than an error.
            if is_link_exists_stderr(&stderr) {
                debug!("Link already exists (skipping): {} -> {}", src, dst);
            } else {
                anyhow::bail!("pw-link failed for {}: {}", channel, stderr.trim());
            }
        }
    }
    Ok(())
}

/// Sets the volume percentage (`0..=100`) on a sink via `pactl set-sink-volume`.
/// Returns `true` if `pactl` exited successfully.
pub(crate) fn set_sink_volume(sink_name: &str, volume_percent: u8) -> bool {
    let clamped = volume_percent.min(MAX_VOLUME_PERCENT);
    Command::new("pactl")
        .args(["set-sink-volume", sink_name, &format!("{}%", clamped)])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Parses `pactl list short sink-inputs` output into a list of stream index IDs.
pub(crate) fn parse_sink_input_indices(sink_inputs_output: &str) -> Vec<u32> {
    sink_inputs_output
        .lines()
        .filter_map(|line| {
            let first_col = line.split('\t').next()?.trim();
            first_col.parse::<u32>().ok()
        })
        .collect()
}

/// Moves all currently active audio streams (sink-inputs) to the specified target sink.
pub(crate) fn move_all_inputs_to(sink_name: &str) -> Result<()> {
    let output = Command::new("pactl")
        .args(["list", "short", "sink-inputs"])
        .output()
        .context("Failed to list sink-inputs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pactl list short sink-inputs failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout)?;
    for index in parse_sink_input_indices(&stdout) {
        // Ignore individual move errors — some streams (e.g. system monitors) may not be movable.
        let _ = Command::new("pactl")
            .args(["move-sink-input", &index.to_string(), sink_name])
            .status();
    }
    Ok(())
}

/// Metadata for an available PipeWire/PulseAudio output sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioSinkInfo {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub state: String,
}

/// Metadata for an active PipeWire/PulseAudio application playback stream (`sink-input`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SinkInputInfo {
    pub index: u32,
    pub sink_index: u32,
    pub sink_name: String,
    pub app_name: String,
    pub app_binary: String,
    pub app_key: String,
    pub media_name: String,
    pub media_title: String,
    pub icon_name: String,
    pub corked: bool,
}

/// Strips surrounding double quotes from a `pactl` property value (`"..."`) and unescapes `\"`.
fn unquote_pactl_prop(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        trimmed.to_string()
    }
}

/// Parses `pactl list sinks` output into structured `AudioSinkInfo` records.
pub(crate) fn parse_sinks_output(output: &str) -> Vec<AudioSinkInfo> {
    let mut sinks = Vec::new();
    let mut current: Option<AudioSinkInfo> = None;
    let mut in_properties = false;

    for raw_line in output.lines() {
        let line = raw_line.trim_end();
        if let Some(rest) = line.strip_prefix("Sink #") {
            if let Some(prev) = current.take() {
                if !prev.name.is_empty() {
                    sinks.push(prev);
                }
            }
            in_properties = false;
            if let Ok(index) = rest.trim().parse::<u32>() {
                current = Some(AudioSinkInfo {
                    index,
                    name: String::new(),
                    description: String::new(),
                    state: String::new(),
                });
            }
            continue;
        }

        let Some(ref mut sink) = current else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed == "Properties:" {
            in_properties = true;
            continue;
        }
        if !in_properties {
            if let Some(v) = trimmed.strip_prefix("State:") {
                sink.state = v.trim().to_string();
            } else if let Some(v) = trimmed.strip_prefix("Name:") {
                sink.name = v.trim().to_string();
            } else if let Some(v) = trimmed.strip_prefix("Description:") {
                sink.description = v.trim().to_string();
            }
        }
    }

    if let Some(last) = current {
        if !last.name.is_empty() {
            sinks.push(last);
        }
    }
    sinks
}

/// Returns `true` if `binary` is a generic runtime wrapper (e.g. `electron`, `python3`, `java`)
/// rather than an application-specific executable, so `application.name` should be preferred as `app_key`.
fn is_generic_wrapper_binary(binary: &str) -> bool {
    let lower = binary.trim().to_ascii_lowercase();
    lower == "electron"
        || lower.starts_with("electron")
        || matches!(
            lower.as_str(),
            "node" | "nodejs" | "python" | "python3" | "java" | "javaw" | "wine64-preloader" | "flatpak-session-helper"
        )
}

/// Parses `pactl list sink-inputs` output into structured `SinkInputInfo` records.
pub(crate) fn parse_sink_inputs_output(
    output: &str,
    sink_index_to_name: &HashMap<u32, String>,
) -> Vec<SinkInputInfo> {
    let mut streams = Vec::new();
    let mut current: Option<SinkInputInfo> = None;
    let mut in_properties = false;

    let finalize = |mut s: SinkInputInfo| -> SinkInputInfo {
        if s.app_key.is_empty() {
            s.app_key = if !s.app_binary.is_empty() && !is_generic_wrapper_binary(&s.app_binary) {
                s.app_binary.clone()
            } else if !s.app_name.is_empty() {
                s.app_name.clone()
            } else if !s.app_binary.is_empty() {
                s.app_binary.clone()
            } else {
                format!("stream-{}", s.index)
            };
        }
        if s.app_name.is_empty() {
            s.app_name = if !s.app_binary.is_empty() {
                s.app_binary.clone()
            } else {
                s.app_key.clone()
            };
        }
        if let Some(name) = sink_index_to_name.get(&s.sink_index) {
            s.sink_name = name.clone();
        }
        s
    };

    for raw_line in output.lines() {
        let line = raw_line.trim_end();
        if let Some(rest) = line.strip_prefix("Sink Input #") {
            if let Some(prev) = current.take() {
                streams.push(finalize(prev));
            }
            in_properties = false;
            if let Ok(index) = rest.trim().parse::<u32>() {
                current = Some(SinkInputInfo {
                    index,
                    sink_index: 0,
                    sink_name: String::new(),
                    app_name: String::new(),
                    app_binary: String::new(),
                    app_key: String::new(),
                    media_name: String::new(),
                    media_title: String::new(),
                    icon_name: String::new(),
                    corked: false,
                });
            }
            continue;
        }

        let Some(ref mut stream) = current else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed == "Properties:" {
            in_properties = true;
            continue;
        }

        if !in_properties {
            if let Some(v) = trimmed.strip_prefix("Sink:") {
                if let Ok(idx) = v.trim().parse::<u32>() {
                    stream.sink_index = idx;
                }
            } else if let Some(v) = trimmed.strip_prefix("Corked:") {
                stream.corked = v.trim().eq_ignore_ascii_case("yes");
            }
        } else if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = unquote_pactl_prop(v);
            match key {
                "application.name" => stream.app_name = val,
                "application.process.binary" => stream.app_binary = val,
                "media.name" => stream.media_name = val,
                "media.title" => stream.media_title = val,
                "application.icon_name" => stream.icon_name = val,
                _ => {}
            }
        }
    }

    if let Some(last) = current {
        streams.push(finalize(last));
    }
    streams
}

/// Lists all available output sinks via `pactl list sinks`.
pub(crate) fn list_sinks() -> Result<Vec<AudioSinkInfo>> {
    let output = Command::new("pactl")
        .args(["list", "sinks"])
        .output()
        .context("Failed to execute pactl list sinks")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pactl list sinks failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_sinks_output(&stdout))
}

/// Lists all active application playback streams (`sink-inputs`) with resolved sink names.
pub(crate) fn list_sink_inputs() -> Result<Vec<SinkInputInfo>> {
    let sinks = list_sinks().unwrap_or_default();
    let sink_map: HashMap<u32, String> = sinks.into_iter().map(|s| (s.index, s.name)).collect();

    let output = Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .context("Failed to execute pactl list sink-inputs")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("pactl list sink-inputs failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_sink_inputs_output(&stdout, &sink_map))
}

/// Moves a single `sink-input` stream to the target sink name.
pub(crate) fn move_sink_input(input_index: u32, sink_name: &str) -> Result<()> {
    let status = Command::new("pactl")
        .args(["move-sink-input", &input_index.to_string(), sink_name])
        .status()
        .with_context(|| {
            format!(
                "Failed to execute pactl move-sink-input {} {}",
                input_index, sink_name
            )
        })?;
    if !status.success() {
        anyhow::bail!(
            "pactl move-sink-input {} {} exited with {}",
            input_index,
            sink_name,
            status
        );
    }
    Ok(())
}

/// Moves all active streams belonging to `app_key` to `sink_name`. Returns count of moved streams.
pub(crate) fn move_app_streams(app_key: &str, sink_name: &str) -> Result<usize> {
    let streams = list_sink_inputs()?;
    let mut moved = 0usize;
    for stream in streams {
        if stream.app_key.eq_ignore_ascii_case(app_key)
            || stream.app_name.eq_ignore_ascii_case(app_key)
        {
            if stream.sink_name != sink_name && move_sink_input(stream.index, sink_name).is_ok() {
                moved += 1;
            }
        }
    }
    Ok(moved)
}

/// Applies saved `app_routing` rules to newly discovered streams without fighting manual per-stream overrides.
pub(crate) fn apply_saved_routing_rules(
    rules: &BTreeMap<String, String>,
    auto_routed_seen: &mut HashMap<u32, String>,
) -> usize {
    if rules.is_empty() {
        auto_routed_seen.clear();
        return 0;
    }

    let Ok(sinks) = list_sinks() else {
        return 0;
    };
    let available_sinks: HashSet<String> = sinks.iter().map(|s| s.name.clone()).collect();
    let sink_map: HashMap<u32, String> = sinks.into_iter().map(|s| (s.index, s.name)).collect();

    let Ok(output) = Command::new("pactl").args(["list", "sink-inputs"]).output() else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    let streams = parse_sink_inputs_output(&String::from_utf8_lossy(&output.stdout), &sink_map);

    let active_ids: HashSet<u32> = streams.iter().map(|s| s.index).collect();
    auto_routed_seen.retain(|id, _| active_ids.contains(id));

    let mut moved = 0usize;
    for stream in streams {
        let target_sink = rules.get(&stream.app_key).or_else(|| {
            rules
                .iter()
                .find(|(k, _)| {
                    k.eq_ignore_ascii_case(&stream.app_key)
                        || k.eq_ignore_ascii_case(&stream.app_name)
                })
                .map(|(_, v)| v)
        });
        let Some(target_sink) = target_sink else {
            continue;
        };
        if !available_sinks.contains(target_sink) {
            continue;
        }
        let already_applied_target = auto_routed_seen
            .get(&stream.index)
            .map(|s| s == target_sink)
            .unwrap_or(false);
        if already_applied_target {
            continue;
        }
        if stream.sink_name != *target_sink {
            if move_sink_input(stream.index, target_sink).is_ok() {
                moved += 1;
                debug!(
                    "Auto-routed stream #{} ({}) -> {}",
                    stream.index, stream.app_key, target_sink
                );
            }
        }
        auto_routed_seen.insert(stream.index, target_sink.clone());
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_arctis_sink_prefers_usb_over_fallback() {
        let sample = "\
42\talsa_output.pci-0000_00_1f.3.analog-stereo\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED
55\tArctis_Game\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE
56\tArctis_Chat\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE
61\talsa_output.steelseries_arctis_nova_7\tPipeWire\ts16le 2ch 48000Hz\tIDLE
62\talsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo\tPipeWire\ts16le 2ch 48000Hz\tRUNNING
";
        assert_eq!(
            parse_arctis_sink(sample),
            Some("alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo".to_string())
        );
    }

    #[test]
    fn test_parse_arctis_sink_fallback_and_ignores_virtual_and_non_nova7_sinks() {
        let only_virtual_or_non_7_at_index_57 = "\
7\tArctis_Game\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE
17\tArctis_Chat\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE
57\talsa_output.usb-SteelSeries_Arctis_5-00.analog-stereo\tPipeWire\ts16le 2ch 48000Hz\tRUNNING
77\talsa_output.usb-SteelSeries_Arctis_Pro-00.analog-stereo\tPipeWire\ts16le 2ch 48000Hz\tIDLE
";
        assert_eq!(parse_arctis_sink(only_virtual_or_non_7_at_index_57), None);

        let fallback_only = "\
61\talsa_output.steelseries_arctis_7_custom\tPipeWire\ts16le 2ch 48000Hz\tIDLE
";
        assert_eq!(
            parse_arctis_sink(fallback_only),
            Some("alsa_output.steelseries_arctis_7_custom".to_string())
        );
    }

    #[test]
    fn test_parse_sink_input_indices() {
        let sample = "\
104\t55\t101\tPipeWire\tfloat32le 2ch 48000Hz
208\t62\t145\tPipeWire\ts16le 2ch 48000Hz
invalid\t62\t145\tPipeWire\ts16le 2ch 48000Hz

";
        assert_eq!(parse_sink_input_indices(sample), vec![104, 208]);
        assert!(parse_sink_input_indices("").is_empty());
    }

    #[test]
    fn test_format_null_sink_args() {
        let args = format_null_sink_args("Arctis_Game", "Arctis Nova 7 Game");
        assert_eq!(
            args,
            r#"{factory.name=support.null-audio-sink node.name=Arctis_Game node.description="Arctis Nova 7 Game" media.class=Audio/Sink monitor.channel-volumes=true object.linger=true audio.position=[FL FR]}"#
        );
    }

    #[test]
    fn test_is_link_exists_stderr() {
        assert!(is_link_exists_stderr("failed to link ports: File exists"));
        assert!(is_link_exists_stderr("link already EXISTS"));
        assert!(!is_link_exists_stderr("No such port"));
    }

    #[test]
    fn test_parse_sinks_and_sink_inputs_output() {
        let sinks_raw = "\
Sink #83
\tState: IDLE
\tName: Arctis_Game
\tDescription: Arctis Nova 7 Game
\tProperties:
\t\tnode.name = \"Arctis_Game\"
Sink #89
\tState: RUNNING
\tName: Arctis_Chat
\tDescription: Arctis Nova 7 Chat
";
        let sinks = parse_sinks_output(sinks_raw);
        assert_eq!(sinks.len(), 2);
        assert_eq!(sinks[0].index, 83);
        assert_eq!(sinks[0].name, "Arctis_Game");
        assert_eq!(sinks[0].description, "Arctis Nova 7 Game");
        assert_eq!(sinks[1].state, "RUNNING");

        let mut sink_map = HashMap::new();
        for s in &sinks {
            sink_map.insert(s.index, s.name.clone());
        }

        let inputs_raw = "\
Sink Input #124
\tDriver: PipeWire
\tSink: 83
\tCorked: no
\tProperties:
\t\tapplication.name = \"Vivaldi\"
\t\tapplication.process.binary = \"vivaldi-bin\"
\t\tmedia.name = \"Playback\"
\t\tmedia.title = \"YouTube Music - Song\"
\t\tapplication.icon_name = \"vivaldi\"
Sink Input #125
\tDriver: PipeWire
\tSink: 89
\tCorked: yes
\tProperties:
\t\tapplication.name = \"Discord\"
\t\tmedia.name = \"WEBRTC VoiceEngine\"
";
        let streams = parse_sink_inputs_output(inputs_raw, &sink_map);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].index, 124);
        assert_eq!(streams[0].sink_name, "Arctis_Game");
        assert_eq!(streams[0].app_key, "vivaldi-bin");
        assert_eq!(streams[0].media_title, "YouTube Music - Song");
        assert!(!streams[0].corked);

        assert_eq!(streams[1].index, 125);
        assert_eq!(streams[1].sink_name, "Arctis_Chat");
        assert_eq!(streams[1].app_key, "Discord");
        assert!(streams[1].corked);
    }

    #[test]
    fn test_generic_wrapper_binary_uses_app_name_as_app_key() {
        let mut sink_map = HashMap::new();
        sink_map.insert(89, "Arctis_Chat".to_string());

        let inputs_raw = "\
Sink Input #1993
\tDriver: PipeWire
\tSink: 89
\tCorked: no
\tProperties:
\t\tapplication.name = \"vesktop\"
\t\tapplication.process.binary = \"electron\"
\t\tmedia.name = \"Playback\"
";
        let streams = parse_sink_inputs_output(inputs_raw, &sink_map);
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].app_binary, "electron");
        assert_eq!(streams[0].app_name, "vesktop");
        assert_eq!(streams[0].app_key, "vesktop");
    }
}
