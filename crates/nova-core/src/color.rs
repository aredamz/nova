use crate::anim::Lerp;

/// Straight-alpha (not premultiplied) colour with sRGB-encoded channels in 0..=1.
///
/// NOVA composites in gamma-encoded sRGB, the same space a browser uses for
/// CSS, so that fades, gradients and blurs look like they do in Remotion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        [q(self.r), q(self.g), q(self.b), q(self.a)]
    }

    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// Accepts `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, `rgb(r,g,b)`,
    /// `rgba(r,g,b,a)` (a in 0..1) and the names `transparent`, `black`, `white`.
    pub fn parse(input: &str) -> Result<Color, String> {
        let s = input.trim();
        match s.to_ascii_lowercase().as_str() {
            "transparent" => return Ok(Color::TRANSPARENT),
            "black" => return Ok(Color::BLACK),
            "white" => return Ok(Color::WHITE),
            _ => {}
        }
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex).ok_or_else(|| format!("invalid hex colour `{input}`"));
        }
        let lower = s.to_ascii_lowercase();
        let body = lower
            .strip_prefix("rgba(")
            .or_else(|| lower.strip_prefix("rgb("))
            .and_then(|r| r.strip_suffix(')'));
        if let Some(body) = body {
            let parts: Vec<&str> = body.split(',').map(str::trim).collect();
            if parts.len() == 3 || parts.len() == 4 {
                let ch = |p: &str| p.parse::<f32>().ok().filter(|v| (0.0..=255.0).contains(v));
                let (r, g, b) = (ch(parts[0]), ch(parts[1]), ch(parts[2]));
                let a = if parts.len() == 4 {
                    parts[3]
                        .parse::<f32>()
                        .ok()
                        .filter(|v| (0.0..=1.0).contains(v))
                } else {
                    Some(1.0)
                };
                if let (Some(r), Some(g), Some(b), Some(a)) = (r, g, b, a) {
                    return Ok(Color {
                        r: r / 255.0,
                        g: g / 255.0,
                        b: b / 255.0,
                        a,
                    });
                }
            }
            return Err(format!("invalid rgb()/rgba() colour `{input}`"));
        }
        Err(format!(
            "unrecognised colour `{input}` (use #RRGGBB, #RRGGBBAA or rgba(r,g,b,a))"
        ))
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nib = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some(Color::rgba8(nib(0)?, nib(1)?, nib(2)?, 255)),
        4 => Some(Color::rgba8(nib(0)?, nib(1)?, nib(2)?, nib(3)?)),
        6 => Some(Color::rgba8(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some(Color::rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

impl Lerp for Color {
    /// Channel-wise in encoded sRGB with straight alpha — the behaviour of
    /// Remotion's `interpolateColors`. `t` may overshoot (springs); the result
    /// is clamped when converted for drawing.
    fn lerp(a: &Color, b: &Color, t: f32) -> Color {
        Color {
            r: a.r + (b.r - a.r) * t,
            g: a.g + (b.g - a.g) * t,
            b: a.b + (b.b - a.b) * t,
            a: (a.a + (b.a - a.a) * t).clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Color::parse("#fff").unwrap(), Color::WHITE);
        assert_eq!(
            Color::parse("#101522").unwrap().to_rgba8(),
            [0x10, 0x15, 0x22, 255]
        );
        assert_eq!(
            Color::parse("#10152280").unwrap().to_rgba8(),
            [0x10, 0x15, 0x22, 0x80]
        );
        assert_eq!(Color::parse("#f00a").unwrap().to_rgba8(), [255, 0, 0, 170]);
    }

    #[test]
    fn parses_functional_forms() {
        assert_eq!(
            Color::parse("rgb(255, 0, 128)").unwrap().to_rgba8(),
            [255, 0, 128, 255]
        );
        assert_eq!(
            Color::parse("rgba(0,0,0,0.5)").unwrap().to_rgba8(),
            [0, 0, 0, 128]
        );
        assert_eq!(Color::parse("transparent").unwrap(), Color::TRANSPARENT);
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "#12",
            "#ggg",
            "red-ish",
            "rgb(1,2)",
            "rgba(0,0,0,2)",
            "rgb(300,0,0)",
        ] {
            assert!(Color::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn lerp_midpoint() {
        let m = Color::lerp(&Color::BLACK, &Color::WHITE, 0.5);
        assert_eq!(m.to_rgba8(), [128, 128, 128, 255]);
    }
}
