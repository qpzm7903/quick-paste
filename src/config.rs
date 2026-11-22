//! 应用配置管理：全局快捷键、主题、粘贴行为、历史记录上限与隐私排除应用。

use std::{fs, io::Write as _, path::Path};

use atomic_write_file::AtomicWriteFile;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const CONFIG_HEADER: &str = "\
# QuickPaste 配置文件
# 修改后保存或直接在应用「设置」界面调整即可。
";

pub const MIN_HISTORY_ITEMS: usize = 50;
pub const MAX_HISTORY_ITEMS: usize = 5000;
pub const MIN_IMAGE_MB: usize = 1;
pub const MAX_IMAGE_MB: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteTrigger {
    #[default]
    DoubleClick,
    SingleClick,
}

impl PasteTrigger {
    pub const ALL: [Self; 2] = [Self::DoubleClick, Self::SingleClick];

    pub fn label(self) -> &'static str {
        match self {
            Self::DoubleClick => "双击条目",
            Self::SingleClick => "单击条目",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub hotkey: String,
    pub theme: ThemePreference,
    pub paste_trigger: PasteTrigger,
    pub restore_clipboard: bool,
    pub record_clipboard: bool,
    pub record_images: bool,
    pub max_history_items: usize,
    pub max_image_mb: usize,
    pub launch_at_login: bool,
    pub position_on_cursor_screen: bool,
    pub hide_on_blur: bool,
    pub excluded_apps: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey().to_owned(),
            theme: ThemePreference::System,
            paste_trigger: PasteTrigger::DoubleClick,
            restore_clipboard: false,
            record_clipboard: true,
            record_images: true,
            max_history_items: 500,
            max_image_mb: 20,
            launch_at_login: false,
            position_on_cursor_screen: true,
            hide_on_blur: false,
            excluded_apps: default_excluded_apps(),
        }
    }
}

impl AppConfig {
    /// 对数值字段进行合理范围钳制与规范化。
    pub fn normalized(mut self) -> Self {
        self.hotkey = self.hotkey.trim().to_owned();
        if self.hotkey.is_empty() {
            self.hotkey = default_hotkey().to_owned();
        }
        self.max_history_items = self
            .max_history_items
            .clamp(MIN_HISTORY_ITEMS, MAX_HISTORY_ITEMS);
        self.max_image_mb = self.max_image_mb.clamp(MIN_IMAGE_MB, MAX_IMAGE_MB);
        self.excluded_apps = self
            .excluded_apps
            .into_iter()
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        self
    }

    /// 判断给定前台应用是否位于隐私排除列表。
    pub fn is_app_excluded(&self, app_name: Option<&str>, app_id: Option<&str>) -> bool {
        let candidates: Vec<String> = [app_name, app_id]
            .into_iter()
            .flatten()
            .map(normalize_app_token)
            .filter(|token| !token.is_empty())
            .collect();
        if candidates.is_empty() {
            return false;
        }

        self.excluded_apps.iter().any(|rule| {
            let rule = normalize_app_token(rule);
            if rule.is_empty() {
                return false;
            }
            candidates.iter().any(|candidate| {
                if rule.chars().count() < 3 {
                    candidate == &rule
                } else {
                    candidate == &rule || candidate.contains(&rule)
                }
            })
        })
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("读写配置文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("配置 YAML 格式无效：{0}")]
    Yaml(#[from] serde_yaml::Error),
}

pub fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+Shift+V"
    } else {
        "Ctrl+Shift+V"
    }
}

pub fn fallback_hotkeys() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["Cmd+Shift+V", "Alt+Shift+V", "Ctrl+Shift+X", "Cmd+Alt+V"]
    } else {
        &["Ctrl+Shift+V", "Ctrl+Shift+X", "Ctrl+Alt+V", "Alt+Shift+V"]
    }
}

pub fn preset_hotkeys() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["Cmd+Shift+V", "Alt+Shift+V", "Ctrl+Shift+X", "Cmd+Alt+V"]
    } else {
        &["Ctrl+Shift+V", "Ctrl+Shift+X", "Ctrl+Alt+V", "Alt+Shift+V"]
    }
}

pub fn default_excluded_apps() -> Vec<String> {
    [
        "1Password",
        "Bitwarden",
        "KeePassXC",
        "KeePass",
        "Keychain Access",
        "com.apple.keychainaccess",
        "Passwords",
        "com.apple.Passwords",
        "LastPass",
        "Dashlane",
        "Enpass",
        "NordPass",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub fn load_or_create(path: &Path) -> Result<AppConfig, ConfigError> {
    if !path.exists() {
        let config = AppConfig::default();
        save(path, &config)?;
        return Ok(config);
    }
    let content = fs::read_to_string(path)?;
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let only_comments = stripped
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'));
    if only_comments {
        return Ok(AppConfig::default());
    }
    let config: AppConfig = serde_yaml::from_str(stripped)?;
    Ok(config.normalized())
}

pub fn save(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let normalized = config.clone().normalized();
    let yaml = serde_yaml::to_string(&normalized)?;
    let content = format!("{CONFIG_HEADER}{yaml}");
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(content.as_bytes())?;
    file.as_file().sync_all()?;
    file.commit()?;
    Ok(())
}

/// 解析类似 `Cmd+Shift+V` / `Ctrl+Alt+X` 的快捷键字符串。
/// 为避免误拦截正常打字，普通按键必须至少包含一个修饰键（或 F1~F24 功能键）。
pub fn parse_hotkey(spec: &str) -> Result<HotKey, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("快捷键不能为空".to_owned());
    }

    let normalized_tokens: Vec<String> = trimmed
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(canonical_hotkey_token)
        .collect();
    if normalized_tokens.is_empty() {
        return Err("快捷键格式无效".to_owned());
    }

    let normalized = normalized_tokens.join("+");
    let hotkey: HotKey = normalized
        .parse()
        .map_err(|error| format!("无法识别快捷键「{trimmed}」：{error}"))?;

    let has_modifier = !(hotkey.mods
        & (Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER))
        .is_empty();
    if !has_modifier && !is_function_key(hotkey.key) {
        return Err("全局快捷键至少需要包含一个修饰键（Cmd/Ctrl/Alt/Shift）".to_owned());
    }

    Ok(hotkey)
}

/// 将热键规范化为跨平台统一存储格式，例如 `Cmd+Shift+V` 或 `Ctrl+Alt+X`。
pub fn canonicalize_hotkey(spec: &str) -> Result<String, String> {
    let hotkey = parse_hotkey(spec)?;
    let mut parts = Vec::new();
    if hotkey.mods.contains(Modifiers::SUPER) {
        parts.push(if cfg!(target_os = "macos") {
            "Cmd".to_owned()
        } else {
            "Super".to_owned()
        });
    }
    if hotkey.mods.contains(Modifiers::CONTROL) {
        parts.push("Ctrl".to_owned());
    }
    if hotkey.mods.contains(Modifiers::ALT) {
        parts.push("Alt".to_owned());
    }
    if hotkey.mods.contains(Modifiers::SHIFT) {
        parts.push("Shift".to_owned());
    }
    parts.push(format_key_code(hotkey.key));
    Ok(parts.join("+"))
}

/// 格式化快捷键用于 UI 展示（macOS 使用 `⌘⇧V` 符号风格）。
pub fn format_hotkey_display(spec: &str) -> String {
    let Ok(hotkey) = parse_hotkey(spec) else {
        return spec.trim().to_owned();
    };

    if cfg!(target_os = "macos") {
        let mut out = String::new();
        if hotkey.mods.contains(Modifiers::CONTROL) {
            out.push('⌃');
        }
        if hotkey.mods.contains(Modifiers::ALT) {
            out.push('⌥');
        }
        if hotkey.mods.contains(Modifiers::SHIFT) {
            out.push('⇧');
        }
        if hotkey.mods.contains(Modifiers::SUPER) {
            out.push('⌘');
        }
        out.push_str(&format_key_code(hotkey.key));
        out
    } else {
        let mut parts = Vec::new();
        if hotkey.mods.contains(Modifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if hotkey.mods.contains(Modifiers::ALT) {
            parts.push("Alt".to_owned());
        }
        if hotkey.mods.contains(Modifiers::SHIFT) {
            parts.push("Shift".to_owned());
        }
        if hotkey.mods.contains(Modifiers::SUPER) {
            parts.push("Win".to_owned());
        }
        parts.push(format_key_code(hotkey.key));
        parts.join("+")
    }
}

fn canonical_hotkey_token(token: &str) -> String {
    match token.trim().to_ascii_lowercase().as_str() {
        "cmd" | "command" | "⌘" | "win" | "meta" | "super" => "Super".to_owned(),
        "ctrl" | "control" | "⌃" => "Control".to_owned(),
        "opt" | "option" | "alt" | "⌥" => "Alt".to_owned(),
        "shift" | "⇧" => "Shift".to_owned(),
        other if other.len() == 1 => {
            let ch = other.chars().next().unwrap();
            if ch.is_ascii_alphabetic() {
                format!("Key{}", ch.to_ascii_uppercase())
            } else if ch.is_ascii_digit() {
                format!("Digit{ch}")
            } else {
                token.trim().to_owned()
            }
        }
        _ => token.trim().to_owned(),
    }
}

fn format_key_code(code: Code) -> String {
    let raw = code.to_string();
    if let Some(ch) = raw.strip_prefix("Key") {
        return ch.to_owned();
    }
    if let Some(digit) = raw.strip_prefix("Digit") {
        return digit.to_owned();
    }
    raw
}

fn is_function_key(code: Code) -> bool {
    matches!(
        code,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
    )
}

fn normalize_app_token(raw: &str) -> String {
    let lowered = raw.trim().to_lowercase();
    lowered.strip_suffix(".exe").unwrap_or(&lowered).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_config_yaml() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.yaml");

        let mut cfg = AppConfig {
            hotkey: "Alt+Shift+V".to_owned(),
            theme: ThemePreference::Dark,
            paste_trigger: PasteTrigger::SingleClick,
            restore_clipboard: true,
            max_history_items: 99_999,
            ..Default::default()
        };
        cfg.excluded_apps.push("MySecretApp.exe".to_owned());

        save(&path, &cfg).expect("save config");
        let loaded = load_or_create(&path).expect("load config");
        assert_eq!(loaded.hotkey, "Alt+Shift+V");
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert_eq!(loaded.paste_trigger, PasteTrigger::SingleClick);
        assert!(loaded.restore_clipboard);
        assert_eq!(loaded.max_history_items, MAX_HISTORY_ITEMS);
        assert!(loaded.is_app_excluded(Some("mysecretapp"), None));
    }

    #[test]
    fn missing_config_creates_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("config.yaml");
        let loaded = load_or_create(&path).expect("load default");
        assert_eq!(loaded, AppConfig::default());
        assert!(path.exists());
    }

    #[test]
    fn parses_and_rejects_hotkeys() {
        let hk = parse_hotkey("Cmd+Shift+V").expect("valid cmd+shift+v");
        assert!(hk.mods.contains(Modifiers::SUPER));
        assert!(hk.mods.contains(Modifiers::SHIFT));
        assert_eq!(hk.key, Code::KeyV);

        let hk2 = parse_hotkey("ctrl + alt + x").expect("valid ctrl+alt+x");
        assert!(hk2.mods.contains(Modifiers::CONTROL));
        assert!(hk2.mods.contains(Modifiers::ALT));
        assert_eq!(hk2.key, Code::KeyX);

        assert!(parse_hotkey("V").is_err());
        assert!(parse_hotkey("").is_err());
        assert!(parse_hotkey("F9").is_ok());
    }

    #[test]
    fn formats_hotkey_display_sensibly() {
        let display = format_hotkey_display("Ctrl+Shift+V");
        #[cfg(target_os = "macos")]
        assert_eq!(display, "⌃⇧V");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(display, "Ctrl+Shift+V");
    }

    #[test]
    fn excluded_apps_match_case_insensitively_and_strip_exe() {
        let cfg = AppConfig::default();
        assert!(cfg.is_app_excluded(Some("1Password 7"), None));
        assert!(cfg.is_app_excluded(Some("keepassxc.exe"), None));
        assert!(cfg.is_app_excluded(None, Some("com.apple.keychainaccess")));
        assert!(!cfg.is_app_excluded(Some("Visual Studio Code"), Some("com.microsoft.VSCode")));
    }
}
