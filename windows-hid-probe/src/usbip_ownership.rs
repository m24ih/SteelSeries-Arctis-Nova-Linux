//! Fail-closed ownership checks for usbip-win2 port listings.

use std::collections::BTreeMap;

/// Parse imported port numbers and remote URLs from `usbip port` output.
pub fn parse_ports(output: &str) -> Result<BTreeMap<u32, String>, String> {
    if !output
        .lines()
        .any(|line| line.trim() == "Imported USB devices")
    {
        return Err("Unrecognized USB/IP port listing".into());
    }
    let mut result = BTreeMap::new();
    let mut current = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix("Port ") {
            let (number, _) = header.split_once(':').ok_or("Malformed port header")?;
            let port: u32 = number.parse().map_err(|_| "Malformed port number")?;
            if current.is_some() {
                return Err("Port without a remote URL".into());
            }
            if result.contains_key(&port) {
                return Err("Duplicate port".into());
            }
            current = Some(port);
        } else if let Some(url) = line.strip_prefix("-> usbip://") {
            let port = current.take().ok_or("Remote URL without a port")?;
            if url.trim().is_empty() {
                return Err("Empty remote URL".into());
            }
            result.insert(port, url.trim().to_owned());
        }
    }
    if current.is_some() {
        return Err("Port without a remote URL".into());
    }
    Ok(result)
}

pub fn owned_port(listing: &str, port: u32, server_port: u16, busid: &str) -> Result<bool, String> {
    let ports = parse_ports(listing)?;
    let Some(remote) = ports.get(&port) else {
        return Ok(false);
    };
    let expected = format!("127.0.0.1:{server_port}/{busid}");
    if remote != &expected {
        return Err(format!(
            "Port {port} now belongs to {remote}; expected {expected}"
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PORTS: &str = "Imported USB devices\nPort 01: device in use\n -> usbip://127.0.0.1:57039/1-1\n -> serial: \nPort 02: device in use\n -> usbip://127.0.0.1:57039/1-2\n";
    #[test]
    fn checks_server_and_bus_identity() {
        assert_eq!(owned_port(PORTS, 1, 57039, "1-1"), Ok(true));
        assert_eq!(owned_port(PORTS, 3, 57039, "1-1"), Ok(false));
        assert!(owned_port(PORTS, 1, 57040, "1-1").is_err());
        assert!(owned_port(PORTS, 1, 57039, "1-2").is_err());
    }
    #[test]
    fn rejects_incomplete_or_ambiguous_listings() {
        assert!(parse_ports("Imported USB devices\nPort 01: in use\n").is_err());
        assert!(parse_ports(
            "Imported USB devices\nPort 01: x\n -> usbip://a\nPort 01: x\n -> usbip://b\n"
        )
        .is_err());
        assert!(parse_ports("Imported USB devices\n -> usbip://a\n").is_err());
        assert!(parse_ports("unexpected output").is_err());
    }
}
