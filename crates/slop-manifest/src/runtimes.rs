// SPDX-License-Identifier: MIT OR Apache-2.0
//! Shared runtimes (DESIGN §6): the Windows runtimes games often need, from an approved list.
//! A release names the ones it needs (by `id`, signed into its manifest); the SLAPorSLOP app installs a
//! missing one before the game starts, once the player agrees, from the vendor's own download,
//! after Windows confirms the file is validly signed by that vendor.
//!
//! The vendors' links always serve the newest version, so files are checked by their signature
//! (`signer`, the certificate's organisation), not a fixed hash.

/// How the SLAPorSLOP app tells a runtime is already installed (Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detect {
    /// A registry value under HKEY_LOCAL_MACHINE that's 1 when installed.
    Registry {
        key: &'static str,
        value: &'static str,
    },
    /// A folder with a subfolder whose name starts with `prefix` (a version).
    Versions {
        dir: &'static str,
        prefix: &'static str,
    },
    /// A file that's there when installed.
    File { path: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    /// The vendor's download.
    pub url: &'static str,
    /// For a quiet install (the installer still asks for admin rights).
    pub args: &'static [&'static str],
    /// The organisation (O=) the file's Authenticode certificate must name.
    pub signer: &'static str,
    pub detect: Detect,
}

pub const RUNTIMES: &[Runtime] = &[
    Runtime {
        id: "vcredist-x64",
        name: "Microsoft Visual C++ Redistributable (64-bit)",
        vendor: "Microsoft",
        url: "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        args: &["/install", "/quiet", "/norestart"],
        signer: "Microsoft Corporation",
        detect: Detect::Registry {
            key: r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64",
            value: "Installed",
        },
    },
    Runtime {
        id: "vcredist-x86",
        name: "Microsoft Visual C++ Redistributable (32-bit)",
        vendor: "Microsoft",
        url: "https://aka.ms/vs/17/release/vc_redist.x86.exe",
        args: &["/install", "/quiet", "/norestart"],
        signer: "Microsoft Corporation",
        detect: Detect::Registry {
            key: r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\x86",
            value: "Installed",
        },
    },
    Runtime {
        id: "dotnet-desktop-8",
        name: ".NET Desktop Runtime 8",
        vendor: "Microsoft",
        url: "https://aka.ms/dotnet/8.0/windowsdesktop-runtime-win-x64.exe",
        args: &["/install", "/quiet", "/norestart"],
        signer: "Microsoft Corporation",
        detect: Detect::Versions {
            dir: r"C:\Program Files\dotnet\shared\Microsoft.WindowsDesktop.App",
            prefix: "8.",
        },
    },
    Runtime {
        id: "dotnet-desktop-9",
        name: ".NET Desktop Runtime 9",
        vendor: "Microsoft",
        url: "https://aka.ms/dotnet/9.0/windowsdesktop-runtime-win-x64.exe",
        args: &["/install", "/quiet", "/norestart"],
        signer: "Microsoft Corporation",
        detect: Detect::Versions {
            dir: r"C:\Program Files\dotnet\shared\Microsoft.WindowsDesktop.App",
            prefix: "9.",
        },
    },
    Runtime {
        id: "directx-9",
        name: "DirectX End-User Runtime (DirectX 9 extras)",
        vendor: "Microsoft",
        url: "https://download.microsoft.com/download/1/7/1/1718CCC4-6315-4D8E-9543-8E28A4E18C4C/dxwebsetup.exe",
        args: &["/Q"],
        signer: "Microsoft Corporation",
        detect: Detect::File {
            path: r"C:\Windows\System32\d3dx9_43.dll",
        },
    },
];

/// The approved runtime with this ID.
pub fn runtime(id: &str) -> Option<&'static Runtime> {
    RUNTIMES.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_well_formed() {
        for r in RUNTIMES {
            assert!(r.url.starts_with("https://"), "{}", r.id);
            assert!(!r.signer.is_empty());
            assert_eq!(runtime(r.id), Some(r));
        }
        let mut ids: Vec<_> = RUNTIMES.iter().map(|r| r.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), RUNTIMES.len());
        assert_eq!(runtime("nope"), None);
    }

    /// The publishing guide lists every runtime by ID.
    #[test]
    fn the_guide_lists_them() {
        let guide = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/public/publishing.md"),
        )
        .unwrap();
        for r in RUNTIMES {
            assert!(guide.contains(&format!("| `{}` |", r.id)), "{}", r.id);
        }
    }
}
