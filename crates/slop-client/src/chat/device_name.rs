// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a device is called in the user's own devices list: the computer's name, so setting chat
//! up needs no question. Only the user sees it (and approves new devices by it); people they chat
//! with see their Slop display name, never a device name.

/// Slop limits device names to 60 characters (crates/slop-api/src/chat.rs).
const MAX: usize = 60;

/// This computer's name (its hostname), or "Slop for <OS>" when it has none worth showing.
pub fn this_device_name() -> String {
    let host = gethostname::gethostname();
    device_name(host.to_str(), os_label(std::env::consts::OS))
}

/// The device name for a hostname: its first label ("brians-mac.local" is "brians-mac"),
/// without control characters, at most 60 characters. Falls back to "Slop for <OS>".
pub fn device_name(hostname: Option<&str>, os: &str) -> String {
    let name: String = hostname
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX)
        .collect();
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("localhost") {
        format!("Slop for {os}")
    } else {
        name.to_string()
    }
}

fn os_label(os: &str) -> &str {
    match os {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_computers_name() {
        assert_eq!(
            device_name(Some("DESKTOP-4F2K9"), "Windows"),
            "DESKTOP-4F2K9"
        );
        assert_eq!(device_name(Some("fedora"), "Linux"), "fedora");
    }

    #[test]
    fn drops_the_domain() {
        assert_eq!(
            device_name(Some("Brians-MacBook-Pro.local"), "macOS"),
            "Brians-MacBook-Pro"
        );
        assert_eq!(device_name(Some("box.example.lan"), "Linux"), "box");
    }

    #[test]
    fn falls_back_when_there_is_no_useful_name() {
        assert_eq!(device_name(None, "Windows"), "Slop for Windows");
        assert_eq!(device_name(Some(""), "Linux"), "Slop for Linux");
        assert_eq!(device_name(Some("  "), "Linux"), "Slop for Linux");
        assert_eq!(device_name(Some("localhost"), "macOS"), "Slop for macOS");
        assert_eq!(device_name(Some(".local"), "macOS"), "Slop for macOS");
    }

    #[test]
    fn fits_slops_limit_without_control_characters() {
        let long = "x".repeat(200);
        assert_eq!(device_name(Some(&long), "Linux").chars().count(), MAX);
        assert_eq!(device_name(Some("my\u{7}pc\n"), "Linux"), "mypc");
    }

    #[test]
    fn this_computer_has_a_name() {
        let name = this_device_name();
        assert!(!name.trim().is_empty());
        assert!(name.chars().count() <= MAX);
    }

    #[test]
    fn os_labels() {
        assert_eq!(os_label("windows"), "Windows");
        assert_eq!(os_label("macos"), "macOS");
        assert_eq!(os_label("linux"), "Linux");
        assert_eq!(os_label("freebsd"), "freebsd");
    }
}
