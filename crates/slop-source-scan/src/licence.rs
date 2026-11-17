// SPDX-License-Identifier: AGPL-3.0-or-later
//! Which licence a text is, SPDX expressions, and which licences are OSI-approved.

use std::sync::LazyLock;

use regex::Regex;

/// OSI-approved licences in common use (opensource.org/licenses), by SPDX ID without
/// `-only`/`-or-later`.
const OSI: &[&str] = &[
    "0BSD",
    "AFL-3.0",
    "AGPL-3.0",
    "Apache-2.0",
    "APSL-2.0",
    "Artistic-2.0",
    "BSD-1-Clause",
    "BSD-2-Clause",
    "BSD-2-Clause-Patent",
    "BSD-3-Clause",
    "BSL-1.0",
    "CDDL-1.0",
    "CECILL-2.1",
    "ECL-2.0",
    "EFL-2.0",
    "EPL-1.0",
    "EPL-2.0",
    "EUPL-1.2",
    "GPL-2.0",
    "GPL-3.0",
    "ISC",
    "LGPL-2.1",
    "LGPL-3.0",
    "LPPL-1.3c",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "MS-PL",
    "MS-RL",
    "MulanPSL-2.0",
    "NCSA",
    "OFL-1.1",
    "OSL-3.0",
    "PostgreSQL",
    "UPL-1.0",
    "Unlicense",
    "Zlib",
    "ZPL-2.1",
    "BlueOak-1.0.0",
    "CC0-1.0",
    "WTFPL",
];

/// An SPDX ID reduced to its family, so GPL-3.0-only, GPL-3.0-or-later and GPL-3.0+ compare equal.
pub(crate) fn family(id: &str) -> String {
    let id = id.trim();
    let id = id
        .strip_suffix("-only")
        .or_else(|| id.strip_suffix("-or-later"))
        .or_else(|| id.strip_suffix('+'))
        .unwrap_or(id);
    id.to_ascii_lowercase()
}

/// The licences in an SPDX expression that aren't on SLAPorSLOP's list of OSI-approved ones
/// (empty when every one is).
pub fn not_osi_approved(expr: &str) -> Vec<String> {
    expression_ids(expr)
        .into_iter()
        .filter(|id| !OSI.iter().any(|o| family(o) == family(id)))
        .collect()
}

/// The licence IDs in an SPDX expression (`MIT OR Apache-2.0`, `GPL-3.0-only WITH x`).
pub fn expression_ids(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for word in expr.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
        if word.is_empty() {
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        match word.to_ascii_uppercase().as_str() {
            "AND" | "OR" => {}
            "WITH" => skip_next = true,
            _ => out.push(word.to_string()),
        }
    }
    out
}

/// Recognises a licence file's text. Covers the licences most apps use; anything else is left
/// for the reviewer.
pub fn identify_licence(text: &str) -> Option<&'static str> {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let has = |s: &str| t.to_ascii_lowercase().contains(&s.to_ascii_lowercase());
    if has("GNU AFFERO GENERAL PUBLIC LICENSE") {
        return Some("AGPL-3.0");
    }
    if has("GNU LESSER GENERAL PUBLIC LICENSE") {
        return Some(if has("Version 2.1") {
            "LGPL-2.1"
        } else {
            "LGPL-3.0"
        });
    }
    if has("GNU GENERAL PUBLIC LICENSE") {
        return Some(if has("Version 2,") || has("Version 2 ") {
            "GPL-2.0"
        } else {
            "GPL-3.0"
        });
    }
    if has("Mozilla Public License") && (has("Version 2.0") || has("version 2.0")) {
        return Some("MPL-2.0");
    }
    if has("Apache License") && has("Version 2.0") {
        return Some("Apache-2.0");
    }
    if has("Eclipse Public License") && has("2.0") {
        return Some("EPL-2.0");
    }
    if has("Boost Software License") {
        return Some("BSL-1.0");
    }
    if has("This is free and unencumbered software released into the public domain") {
        return Some("Unlicense");
    }
    if has("CC0 1.0") || has("Creative Commons Legal Code CC0") {
        return Some("CC0-1.0");
    }
    if has("Permission is hereby granted, free of charge, to any person obtaining a copy") {
        return Some(
            if has("The above copyright notice and this permission notice shall be included") {
                "MIT"
            } else {
                "MIT-0"
            },
        );
    }
    if has("Permission to use, copy, modify, and/or distribute this software for any purpose") {
        return Some(
            if has("with or without fee is hereby granted") && has("copyright notice") {
                "ISC"
            } else {
                "0BSD"
            },
        );
    }
    if has("Redistribution and use in source and binary forms") {
        return Some(
            if has("Neither the name") || has("may be used to endorse") {
                "BSD-3-Clause"
            } else {
                "BSD-2-Clause"
            },
        );
    }
    if has("This software is provided 'as-is'")
        && has("Altered source versions must be plainly marked")
    {
        return Some("Zlib");
    }
    None
}

pub(crate) fn is_licence_file(path: &str) -> bool {
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (Some(d), n),
        None => (None, path),
    };
    let upper = name.to_ascii_uppercase();
    match dir {
        None => ["LICENSE", "LICENCE", "COPYING", "UNLICENSE"]
            .iter()
            .any(|p| upper.starts_with(p)),
        // REUSE keeps one file per licence in LICENSES/.
        Some("LICENSES") => true,
        _ => false,
    }
}

pub(crate) static SPDX_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"SPDX-License-Identifier:\s*([A-Za-z0-9.+\-() ]+?)\s*(\*/|-->|#|$)").unwrap()
});
