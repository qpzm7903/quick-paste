//! 跨平台开机自启封装（macOS LaunchAgent / Windows 注册表）。

use std::path::PathBuf;

use auto_launch::AutoLaunchBuilder;

const APP_NAME: &str = "QuickPaste";
const BUNDLE_ID: &str = "com.g-ai-002.quick-paste";

fn build_auto_launch() -> Result<auto_launch::AutoLaunch, String> {
    let app_path = resolve_app_path()?;
    let path_str = app_path.to_string_lossy().into_owned();

    let mut builder = AutoLaunchBuilder::new();
    builder.set_app_name(APP_NAME);
    builder.set_app_path(&path_str);
    #[cfg(target_os = "macos")]
    {
        builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
        builder.set_bundle_identifiers(&[BUNDLE_ID]);
    }
    #[cfg(target_os = "windows")]
    {
        builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    }

    builder
        .build()
        .map_err(|e| format!("初始化开机自启配置失败：{e}"))
}

fn resolve_app_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取程序路径：{e}"))?;

    #[cfg(target_os = "macos")]
    {
        // 如果运行在 *.app/Contents/MacOS/<bin> 中，优先注册整个 .app bundle 路径。
        if let Some(macos_dir) = exe.parent()
            && macos_dir.file_name().is_some_and(|n| n == "MacOS")
            && let Some(contents_dir) = macos_dir.parent()
            && contents_dir.file_name().is_some_and(|n| n == "Contents")
            && let Some(app_bundle) = contents_dir.parent()
            && app_bundle
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            return Ok(app_bundle.to_path_buf());
        }
    }

    Ok(exe)
}

pub fn sync_launch_at_login(enabled: bool) -> Result<(), String> {
    let auto = build_auto_launch()?;
    let currently_enabled = auto.is_enabled().unwrap_or(false);
    if enabled && !currently_enabled {
        auto.enable()
            .map_err(|e| format!("开启开机自启失败：{e}"))?;
    } else if !enabled && currently_enabled {
        auto.disable()
            .map_err(|e| format!("关闭开机自启失败：{e}"))?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn is_launch_at_login_enabled() -> bool {
    build_auto_launch()
        .and_then(|auto| auto.is_enabled().map_err(|e| e.to_string()))
        .unwrap_or(false)
}
