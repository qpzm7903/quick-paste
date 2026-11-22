//! 富内容类型识别与颜色/链接/时间/容量格式化辅助。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    #[default]
    Text,
    Link,
    Color,
    Code,
    Image,
}

impl ContentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Link => "链接",
            Self::Color => "颜色",
            Self::Code => "代码",
            Self::Image => "图片",
        }
    }

    pub fn search_keywords(self) -> &'static str {
        match self {
            Self::Text => "text 文本 纯文本",
            Self::Link => "link url http https 链接 网址",
            Self::Color => "color hex rgb rgba hsl 颜色 色值",
            Self::Code => "code json sql shell 代码 命令",
            Self::Image => "image img pic png 截图 图片 图像",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorValue {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorValue {
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    pub fn to_rgba_u32(self) -> u32 {
        (u32::from(self.r) << 24)
            | (u32::from(self.g) << 16)
            | (u32::from(self.b) << 8)
            | u32::from(self.a)
    }

    pub fn to_rgb_string(self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            let alpha = f32::from(self.a) / 255.0;
            format!("rgba({}, {}, {}, {:.2})", self.r, self.g, self.b, alpha)
        }
    }
}

/// 识别一段纯文本的语义类型。
pub fn classify_text(text: &str) -> ContentKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ContentKind::Text;
    }

    if !trimmed.contains('\n') {
        if parse_color(trimmed).is_some() {
            return ContentKind::Color;
        }
        if is_http_url(trimmed) {
            return ContentKind::Link;
        }
    }

    if looks_like_code(trimmed) {
        return ContentKind::Code;
    }

    ContentKind::Text
}

pub fn is_http_url(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    let Some(rest) = rest else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && (host.contains('.') || host.starts_with("localhost"))
}

pub fn extract_domain(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.strip_prefix("www.").unwrap_or(host).to_owned())
}

/// 解析 CSS Hex / RGB / RGBA / HSL / HSLA 颜色字符串。
pub fn parse_color(text: &str) -> Option<ColorValue> {
    let trimmed = text.trim();
    if let Some(hex) = trimmed.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        return parse_rgb_inner(inner, false);
    }
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_rgb_inner(inner, true);
    }
    if let Some(inner) = lower.strip_prefix("hsl(").and_then(|s| s.strip_suffix(')')) {
        return parse_hsl_inner(inner, false);
    }
    if let Some(inner) = lower
        .strip_prefix("hsla(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_hsl_inner(inner, true);
    }
    None
}

fn parse_hex_color(hex: &str) -> Option<ColorValue> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = hex.as_bytes();
    match bytes.len() {
        3 => Some(ColorValue {
            r: expand_nibble(bytes[0])?,
            g: expand_nibble(bytes[1])?,
            b: expand_nibble(bytes[2])?,
            a: 255,
        }),
        4 => Some(ColorValue {
            r: expand_nibble(bytes[0])?,
            g: expand_nibble(bytes[1])?,
            b: expand_nibble(bytes[2])?,
            a: expand_nibble(bytes[3])?,
        }),
        6 => Some(ColorValue {
            r: u8::from_str_radix(&hex[0..2], 16).ok()?,
            g: u8::from_str_radix(&hex[2..4], 16).ok()?,
            b: u8::from_str_radix(&hex[4..6], 16).ok()?,
            a: 255,
        }),
        8 => Some(ColorValue {
            r: u8::from_str_radix(&hex[0..2], 16).ok()?,
            g: u8::from_str_radix(&hex[2..4], 16).ok()?,
            b: u8::from_str_radix(&hex[4..6], 16).ok()?,
            a: u8::from_str_radix(&hex[6..8], 16).ok()?,
        }),
        _ => None,
    }
}

fn expand_nibble(ascii: u8) -> Option<u8> {
    let v = (ascii as char).to_digit(16)? as u8;
    Some((v << 4) | v)
}

fn parse_rgb_inner(inner: &str, expect_alpha: bool) -> Option<ColorValue> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if expect_alpha && parts.len() != 4 {
        return None;
    }
    if !expect_alpha && !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }

    let r = parse_channel(parts[0])?;
    let g = parse_channel(parts[1])?;
    let b = parse_channel(parts[2])?;
    let a = if let Some(alpha) = parts.get(3) {
        parse_alpha(alpha)?
    } else {
        255
    };

    Some(ColorValue { r, g, b, a })
}

fn parse_channel(raw: &str) -> Option<u8> {
    if let Some(pct) = raw.strip_suffix('%') {
        let value: f32 = pct.trim().parse().ok()?;
        if !(0.0..=100.0).contains(&value) {
            return None;
        }
        return Some(((value / 100.0) * 255.0).round() as u8);
    }
    raw.parse::<u16>()
        .ok()
        .filter(|&v| v <= 255)
        .map(|v| v as u8)
}

fn parse_alpha(raw: &str) -> Option<u8> {
    if let Some(pct) = raw.strip_suffix('%') {
        let value: f32 = pct.trim().parse().ok()?;
        if !(0.0..=100.0).contains(&value) {
            return None;
        }
        return Some(((value / 100.0) * 255.0).round() as u8);
    }
    let value: f32 = raw.parse().ok()?;
    if !(0.0..=1.0).contains(&value) {
        return None;
    }
    Some((value * 255.0).round() as u8)
}

fn parse_hsl_inner(inner: &str, expect_alpha: bool) -> Option<ColorValue> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if expect_alpha && parts.len() != 4 {
        return None;
    }
    if !expect_alpha && !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }

    let hue: f32 = parts[0]
        .strip_suffix("deg")
        .unwrap_or(parts[0])
        .trim()
        .parse()
        .ok()?;
    let sat: f32 = parts[1].strip_suffix('%')?.trim().parse().ok()?;
    let light: f32 = parts[2].strip_suffix('%')?.trim().parse().ok()?;
    if !(0.0..=100.0).contains(&sat) || !(0.0..=100.0).contains(&light) {
        return None;
    }
    let alpha = if let Some(a) = parts.get(3) {
        parse_alpha(a)?
    } else {
        255
    };

    let (r, g, b) = hsl_to_rgb(hue, sat / 100.0, light / 100.0);
    Some(ColorValue { r, g, b, a: alpha })
}

fn hsl_to_rgb(hue: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = hue.rem_euclid(360.0) / 360.0;
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        (hue_to_rgb(h + 1.0 / 3.0) * 255.0).round() as u8,
        (hue_to_rgb(h) * 255.0).round() as u8,
        (hue_to_rgb(h - 1.0 / 3.0) * 255.0).round() as u8,
    )
}

fn looks_like_code(trimmed: &str) -> bool {
    if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']')))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return true;
    }

    let first_line = trimmed.lines().next().unwrap_or("");
    let cli_prefixes = [
        "git ", "cargo ", "npm ", "pnpm ", "yarn ", "bun ", "docker ", "kubectl ", "curl ",
        "brew ", "ssh ", "rsync ", "python ", "python3 ", "uv ", "go ", "rustc ",
    ];
    if !trimmed.contains('\n')
        && cli_prefixes
            .iter()
            .any(|prefix| first_line.starts_with(prefix))
    {
        return true;
    }

    let line_count = trimmed.lines().count();
    if line_count < 2 {
        return false;
    }

    let code_tokens = [
        "fn ",
        "pub fn ",
        "impl ",
        "struct ",
        "enum ",
        "use ",
        "import ",
        "export ",
        "const ",
        "let ",
        "var ",
        "function ",
        "def ",
        "class ",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "CREATE TABLE",
        "#!/",
        "=>",
        "->",
        "::",
        "```",
    ];
    let has_token = code_tokens.iter().any(|token| trimmed.contains(token));
    let has_braces_or_semi =
        (trimmed.contains('{') && trimmed.contains('}')) || trimmed.contains(';');
    has_token
        || (has_braces_or_semi
            && trimmed
                .lines()
                .any(|line| line.starts_with("  ") || line.starts_with('\t')))
}

pub fn format_relative_time(unix_secs: u64, now_secs: u64) -> String {
    let diff = now_secs.saturating_sub(unix_secs);
    if diff < 60 {
        "刚刚".to_owned()
    } else if diff < 3600 {
        format!("{} 分钟前", diff / 60)
    } else if diff < 86_400 {
        format!("{} 小时前", diff / 3600)
    } else if diff < 172_800 {
        "昨天".to_owned()
    } else {
        format!("{} 天前", diff / 86_400)
    }
}

pub fn format_byte_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    }
}

pub fn char_and_line_summary(text: &str) -> String {
    let chars = text.chars().count();
    let lines = text.lines().count().max(1);
    format!("{chars} 字符 · {lines} 行")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_rgb_and_hsl_colors() {
        assert_eq!(
            parse_color("#0af"),
            Some(ColorValue {
                r: 0,
                g: 170,
                b: 255,
                a: 255
            })
        );
        assert_eq!(
            parse_color("#1e293b"),
            Some(ColorValue {
                r: 30,
                g: 41,
                b: 59,
                a: 255
            })
        );
        assert_eq!(
            parse_color("rgba(255, 128, 0, 0.5)"),
            Some(ColorValue {
                r: 255,
                g: 128,
                b: 0,
                a: 128
            })
        );
        assert_eq!(
            parse_color("hsl(0, 100%, 50%)"),
            Some(ColorValue {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
        assert!(parse_color("#xyz").is_none());
        assert!(parse_color("rgb(300, 0, 0)").is_none());
    }

    #[test]
    fn classifies_links_colors_code_and_plain_text() {
        assert_eq!(classify_text("#3b82f6"), ContentKind::Color);
        assert_eq!(
            classify_text("https://github.com/longbridge/gpui-kit"),
            ContentKind::Link
        );
        assert_eq!(
            extract_domain("https://www.github.com/longbridge/gpui-kit").as_deref(),
            Some("github.com")
        );
        assert_eq!(classify_text(r#"{"hello": "world"}"#), ContentKind::Code);
        assert_eq!(classify_text("cargo test --release"), ContentKind::Code);
        assert_eq!(
            classify_text("fn main() {\n    println!(\"hi\");\n}"),
            ContentKind::Code
        );
        assert_eq!(classify_text("明天下午三点开会"), ContentKind::Text);
    }

    #[test]
    fn formats_time_and_bytes() {
        assert_eq!(format_relative_time(1000, 1020), "刚刚");
        assert_eq!(format_relative_time(1000, 1300), "5 分钟前");
        assert_eq!(format_relative_time(1000, 8200), "2 小时前");
        assert_eq!(format_relative_time(1000, 90_000), "昨天");
        assert_eq!(format_relative_time(1000, 300_000), "3 天前");

        assert_eq!(format_byte_size(512), "512 B");
        assert_eq!(format_byte_size(2048), "2.0 KB");
        assert_eq!(format_byte_size(3 * 1024 * 1024), "3.0 MB");
    }
}
