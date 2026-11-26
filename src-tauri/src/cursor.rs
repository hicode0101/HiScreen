//! Mouse-cursor compositing for captured frames (Windows).
//!
//! DXGI desktop duplication hands out the desktop image *without* the mouse
//! pointer — the cursor travels as separate pointer metadata that scrap
//! ignores, so viewers would otherwise never see it. `draw_cursor` reads the
//! current cursor through Win32, decodes its bitmap (color with alpha, or the
//! classic AND/XOR mask pair) and blends it onto the BGRA frame right before
//! JPEG encoding. Non-Windows platforms currently share without a cursor.

/// Blend the system cursor onto a BGRA frame. `origin` is the captured
/// monitor's top-left corner in virtual-screen coordinates and is used to
/// translate the cursor's screen position into frame coordinates.
pub fn draw_cursor(frame: &mut [u8], w: i32, h: i32, origin: (i32, i32)) {
    #[cfg(windows)]
    imp::draw_cursor(frame, w, h, origin);
    #[cfg(not(windows))]
    let _ = (frame, w, h, origin);
}

/// Top-left corner of the display with the given enumeration index in
/// virtual-screen coordinates (primary first), or `None` when unavailable.
#[cfg(windows)]
pub fn monitor_origin(index: usize) -> Option<(i32, i32)> {
    imp::monitor_origin(index)
}

#[cfg(not(windows))]
pub fn monitor_origin(_index: usize) -> Option<(i32, i32)> {
    None
}

#[cfg(windows)]
mod imp {
    use std::mem;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDIBits,
        GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP,
        HDC, HMONITOR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, HICON, ICONINFO,
    };

    /// A decoded BGRA sprite in top-down row order.
    #[derive(Clone)]
    struct Sprite {
        w: usize,
        h: usize,
        px: Vec<u8>,
    }

    /// Decode a GDI bitmap into a top-down BGRA buffer via GetDIBits
    /// (monochrome and paletted bitmaps are converted by the request).
    /// Dimensions come from GetObjectW; the pixels are fetched through a
    /// memory DC (the screen DC is not reliable for this on all systems).
    fn decode_bitmap(hbm: HBITMAP) -> Option<Sprite> {
        if hbm.is_invalid() {
            return None;
        }
        let mut bm = BITMAP::default();
        unsafe {
            if GetObjectW(
                hbm.into(),
                mem::size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut _ as *mut core::ffi::c_void),
            ) == 0
            {
                return None;
            }
            let w = bm.bmWidth.max(0) as usize;
            let h = bm.bmHeight.max(0) as usize;
            if w == 0 || h == 0 {
                return None;
            }
            let hdc = CreateCompatibleDC(None);
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // Negative height = top-down row order (what the frame uses).
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32, // request conversion to BGRA
                ..Default::default()
            };
            let mut px = vec![0u8; w * h * 4];
            let ok = GetDIBits(
                hdc,
                hbm,
                0,
                h as u32,
                Some(px.as_mut_ptr().cast()),
                &mut bi,
                DIB_RGB_COLORS,
            );
            let _ = DeleteDC(hdc);
            if ok == 0 {
                return None;
            }
            Some(Sprite { w, h, px })
        }
    }

    /// Blend the sprite onto the frame at (dx, dy), honouring per-pixel
    /// alpha and, for mono cursors, the AND/XOR mask pair:
    /// destination = (dest AND and) XOR xor.
    fn blend(
        frame: &mut [u8],
        fw: usize,
        fh: usize,
        sprite: &Sprite,
        dx: i32,
        dy: i32,
        mask_and: Option<&Sprite>,
        mask_xor: Option<&Sprite>,
    ) {
        if dx < 0 || dy < 0 {
            return;
        }
        let (dst_x, dst_y) = (dx as usize, dy as usize);
        // True-alpha cursors shape transparency with the alpha channel alone
        // (their AND mask is conventionally all-black); legacy ones without
        // alpha rely on the AND mask for transparency.
        let alpha_used = sprite.px.chunks_exact(4).any(|p| p[3] > 0);
        for row in 0..sprite.h {
            let fy = dst_y + row;
            if fy >= fh {
                break;
            }
            for col in 0..sprite.w {
                let fx = dst_x + col;
                if fx >= fw {
                    break;
                }
                let si = (row * sprite.w + col) * 4;
                let fi = (fy * fw + fx) * 4;
                let (sb, sg, sr, sa) = (
                    sprite.px[si] as u32,
                    sprite.px[si + 1] as u32,
                    sprite.px[si + 2] as u32,
                    sprite.px[si + 3] as u32,
                );

                if let Some(xor) = mask_xor {
                    // Mono cursor (no color bitmap).
                    let and_on = mask_and
                        .map(|m| m.px[(row * m.w + col) * 4] > 127)
                        .unwrap_or(false);
                    let xor_on = xor.px[si] > 127;
                    if and_on && !xor_on {
                        continue; // transparent hole in the mask
                    }
                    if !and_on {
                        let v = if xor_on { 255u8 } else { 0u8 };
                        frame[fi] = v;
                        frame[fi + 1] = v;
                        frame[fi + 2] = v;
                    } else {
                        // invert the destination pixel
                        frame[fi] = 255 - frame[fi];
                        frame[fi + 1] = 255 - frame[fi + 1];
                        frame[fi + 2] = 255 - frame[fi + 2];
                    }
                    continue;
                }

                // Color cursor: alpha-channel sprites blend src-over; legacy
                // ones without alpha fall back to the AND mask (masked-out =
                // transparent, else opaque).
                let and_transparent = mask_and
                    .map(|m| m.px[(row * m.w + col) * 4] > 127)
                    .unwrap_or(false);
                let alpha: u32 = if alpha_used {
                    sa
                } else if and_transparent {
                    0
                } else {
                    255
                };
                if alpha >= 255 {
                    frame[fi] = sb as u8;
                    frame[fi + 1] = sg as u8;
                    frame[fi + 2] = sr as u8;
                } else if alpha > 0 {
                    frame[fi] = ((sb * alpha + frame[fi] as u32 * (255 - alpha)) / 255) as u8;
                    frame[fi + 1] =
                        ((sg * alpha + frame[fi + 1] as u32 * (255 - alpha)) / 255) as u8;
                    frame[fi + 2] =
                        ((sr * alpha + frame[fi + 2] as u32 * (255 - alpha)) / 255) as u8;
                }
            }
        }
    }

    pub fn draw_cursor(frame: &mut [u8], fw: i32, fh: i32, origin: (i32, i32)) {
        unsafe {
            let mut ci = CURSORINFO::default();
            ci.cbSize = mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci).is_err() {
                return;
            }
            if ci.flags != CURSOR_SHOWING {
                return; // cursor hidden system-wide
            }
            let mut ii = ICONINFO::default();
            // HCURSOR and HICON share the handle representation.
            if GetIconInfo(HICON(ci.hCursor.0), &mut ii).is_err() {
                return;
            }
            let color = decode_bitmap(ii.hbmColor);
            let mask = decode_bitmap(ii.hbmMask);
            // ICONINFO ownership: the two bitmaps must be released by us.
            let _ = DeleteObject(ii.hbmColor.into());
            let _ = DeleteObject(ii.hbmMask.into());

            let Some(mask) = mask else { return };
            let (sprite, mask_and, mask_xor) = if color.is_none() {
                // Mono cursor: the mask bitmap stacks AND rows on XOR rows.
                let half = mask.h / 2;
                let and = Sprite {
                    w: mask.w,
                    h: half,
                    px: mask.px[..half * mask.w * 4].to_vec(),
                };
                let xor = Sprite {
                    w: mask.w,
                    h: half,
                    px: mask.px[half * mask.w * 4..].to_vec(),
                };
                (and.clone(), Some(and), Some(xor))
            } else {
                (color.unwrap(), Some(mask), None)
            };

            let cx = ci.ptScreenPos.x - origin.0 - ii.xHotspot as i32;
            let cy = ci.ptScreenPos.y - origin.1 - ii.yHotspot as i32;
            blend(
                frame,
                fw as usize,
                fh as usize,
                &sprite,
                cx,
                cy,
                mask_and.as_ref(),
                mask_xor.as_ref(),
            );
        }
    }

    /// Enumerate monitors in virtual-screen coordinates and order them
    /// primary-first (origin 0,0), then top-to-bottom / left-to-right —
    /// matching how the UI indexes displays (scrap treats index 0 as primary).
    pub fn monitor_origin(index: usize) -> Option<(i32, i32)> {
        unsafe extern "system" fn collect(
            _hmon: HMONITOR,
            _hdc: HDC,
            rect: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let out = &mut *(lparam.0 as *mut Vec<(i32, i32)>);
            let r = unsafe { &*rect };
            out.push((r.left, r.top));
            BOOL(1)
        }
        let mut origins: Vec<(i32, i32)> = Vec::new();
        unsafe {
            // Returns FALSE only when the callback stopped the enumeration.
            if !EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(&mut origins as *mut _ as isize),
            )
            .as_bool()
            {
                return None;
            }
        }
        origins.sort_by_key(|&(x, y)| (y, x));
        if let Some(pos) = origins.iter().position(|&o| o == (0, 0)) {
            let primary = origins.remove(pos);
            origins.insert(0, primary);
        }
        origins.get(index).copied()
    }
}
