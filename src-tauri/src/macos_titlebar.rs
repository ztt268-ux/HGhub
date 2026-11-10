//! macOS 标题栏：按系统红绿灯的实际尺寸计算垂直居中位置。
//!
//! Tauri 2 只能在创建窗口时设置 `trafficLightPosition`，之后由 tao 在重绘、调整大小、
//! 退出全屏时自动重新应用（tao::platform_impl::macos::view::inset_traffic_lights）。
//! 因此主窗口不再由配置自动创建（tauri.macos.conf.json 中 `create: false`），
//! 而是在 setup 中先测量按钮几何、算出 y，再用这个 y 创建窗口。

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tauri::{LogicalPosition, WebviewWindowBuilder};

const MAIN_WINDOW: &str = "main";

/// 前端标题栏高度（逻辑像素），须与 src/index.css 中 `.win-titlebar` 的 `height` 保持一致。
const TITLEBAR_HEIGHT: f64 = 44.0;

/// 系统红绿灯按钮的几何信息（逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ButtonMetrics {
    /// 按钮高度
    height: f64,
    /// 按钮底边到标题栏视图底边的距离
    bottom: f64,
}

/// 用一个不显示的探测窗口读取当前系统版本下红绿灯的尺寸与默认位置。
/// 窗口样式与 `titleBarStyle: Overlay` + `hiddenTitle: true` 一致。
fn measure_buttons(mtm: MainThreadMarker) -> Option<ButtonMetrics> {
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 300.0));
    // SAFETY: 在主线程创建；defer=true 不分配窗口后备存储，也从不显示。
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    // SAFETY: 窗口由 Retained 持有并在作用域结束时释放，不能再由 close 自动释放。
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);

    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    // SAFETY: 仅读取父视图，不持有到窗口释放之后。
    let parent = unsafe { close.superview() }?;
    // 翻转坐标系下 tao 的定位方式不再适用，交给配置中的固定值兜底。
    if parent.isFlipped() {
        return None;
    }
    let frame = close.frame();
    Some(ButtonMetrics {
        height: frame.size.height,
        bottom: frame.origin.y,
    })
}

/// 计算让红绿灯中心与标题栏中心对齐的 `trafficLightPosition.y`。
///
/// tao 把标题栏容器高度设为 `按钮高 + y`（顶边贴窗口顶部），按钮距容器底部仍是系统默认的
/// `bottom`，所以按钮中心距窗口顶部 = `y - bottom + 按钮高 / 2`。令其等于
/// `titlebar_height / 2` 解出 y；取整到整点，避免 1x 屏幕上按钮发虚。
fn centered_y(metrics: ButtonMetrics, titlebar_height: f64) -> f64 {
    (titlebar_height / 2.0 - metrics.height / 2.0 + metrics.bottom).round()
}

/// 创建主窗口，红绿灯 x 沿用配置，y 按实测按钮尺寸计算；测量失败时沿用配置值。
pub fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN_WINDOW)
        .cloned()
    else {
        return Ok(());
    };

    let mut builder = WebviewWindowBuilder::from_config(app.handle(), &config)?;
    let metrics = MainThreadMarker::new().and_then(measure_buttons);
    if let Some(metrics) = metrics {
        let x = config.traffic_light_position.as_ref().map_or(14.0, |p| p.x);
        builder = builder
            .traffic_light_position(LogicalPosition::new(x, centered_y(metrics, TITLEBAR_HEIGHT)));
    }
    builder.build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_buttons_in_titlebar() {
        // macOS 27 实测：按钮 14pt，默认标题栏 32pt，按钮距底 9pt
        let measured = ButtonMetrics { height: 14.0, bottom: 9.0 };
        let y = centered_y(measured, 44.0);
        assert_eq!(y, 24.0);
        // 代回 tao 的布局：按钮中心 = y - bottom + height / 2 = 标题栏中心
        assert_eq!(y - measured.bottom + measured.height / 2.0, 22.0);
        assert_eq!(centered_y(measured, 52.0), 28.0);
        // 按钮尺寸 / 默认边距不同的系统版本得到不同的 y
        let other = ButtonMetrics { height: 16.0, bottom: 6.0 };
        assert_eq!(centered_y(other, 44.0), 20.0);
        // 奇数差值取整到整点
        assert_eq!(centered_y(ButtonMetrics { height: 13.0, bottom: 9.0 }, 44.0), 25.0);
    }
}
