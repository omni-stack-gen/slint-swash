// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use alloc::boxed::Box;

use i_slint_common::sharedfontique::{self, fontique};
use i_slint_core::lengths::ScaleFactor;

use super::super::PhysicalLength;
use super::vectorfont::VectorFont;

pub fn match_font(
    request: &super::FontRequest,
    scale_factor: super::ScaleFactor,
) -> Option<VectorFont> {
    if request.family.is_some() {
        let weight = request.weight.unwrap_or(400);
        let requested_pixel_size: PhysicalLength =
            (request.pixel_size.unwrap_or(super::DEFAULT_FONT_SIZE).cast() * scale_factor).cast();

        if let Some(font) = request.query_fontique() {
            Some(VectorFont::new(font.blob, font.index, requested_pixel_size, weight))
        } else {
            None
        }
    } else {
        None
    }
}

pub fn fallbackfont(font_request: &super::FontRequest, scale_factor: ScaleFactor) -> VectorFont {
    let weight = font_request.weight.unwrap_or(400);
    let requested_pixel_size: PhysicalLength =
        (font_request.pixel_size.unwrap_or(super::DEFAULT_FONT_SIZE).cast() * scale_factor).cast();

    let font = font_request.query_fontique().unwrap();
    VectorFont::new(font.blob, font.index, requested_pixel_size, weight)
}

pub fn register_font_from_memory(data: &'static [u8]) -> Result<(), Box<dyn std::error::Error>> {
    sharedfontique::get_collection().register_fonts(data.to_vec().into(), None);
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
pub fn register_font_from_path(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let requested_path = path.canonicalize().unwrap_or_else(|_| path.into());
    static REGISTERED_FONT_PATHS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    register_font_path_once(
        &requested_path,
        REGISTERED_FONT_PATHS.get_or_init(Default::default),
        |requested_path| {
            let contents = std::fs::read(requested_path)?;
            sharedfontique::get_collection().register_fonts(contents.into(), None);
            Ok(())
        },
    )
}

#[cfg(not(target_family = "wasm"))]
fn register_font_path_once(
    path: &std::path::Path,
    registered: &std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
    register: impl FnOnce(&std::path::Path) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut registered = registered.lock().unwrap_or_else(|poison| poison.into_inner());
    if registered.contains(path) {
        return Ok(());
    }
    register(path)?;
    registered.insert(path.to_owned());
    Ok(())
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn font_path_is_registered_once_and_failures_are_retryable() {
        let registered = Default::default();
        let calls = AtomicUsize::new(0);
        let path = std::path::Path::new("/verified-assets/customer.ttf");

        register_font_path_once(path, &registered, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        register_font_path_once(path, &registered, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        let retry_path = std::path::Path::new("/verified-assets/retry.ttf");
        assert!(register_font_path_once(retry_path, &registered, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::other("temporary read failure").into())
        })
        .is_err());
        register_font_path_once(retry_path, &registered, |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}
