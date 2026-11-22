//! 虚拟列表的平滑滚动。
//!
//! GPUI 原生滚轮会直接修改 offset；这里先把滚轮增量累积成目标位置，再按帧指数插值。

use std::time::Instant;

use gpui_kit::{
    IsZero as _, Pixels, Point, ScrollEvent, ScrollStrategy, TouchPhase, Window,
    component::VirtualListScrollHandle, px,
};

const SMOOTHING_SECONDS: f32 = 0.11;
const SETTLE_DISTANCE: f32 = 0.5;
const MAX_FRAME_SECONDS: f32 = 0.1;
/// 单次滚轮最多滚动可视区一页，并保留部分重叠。
const PAGE_MIN_FRACTION: f32 = 0.875;
const PAGE_MAX_OVERLAP: f32 = 40.0;

#[derive(Clone, Copy)]
struct ScrollAnimation {
    target: Pixels,
    last_offset: Pixels,
    last_frame_at: Instant,
}

#[derive(Default)]
pub(crate) struct SmoothScroll {
    animation: Option<ScrollAnimation>,
    frame_scheduled: bool,
    touch_gesture_active: bool,
}

impl SmoothScroll {
    pub(crate) fn reset(&mut self) {
        self.animation = None;
        self.frame_scheduled = false;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.animation.is_some() || self.touch_gesture_active
    }

    pub(crate) fn ensure_item_visible(
        &mut self,
        scroll_handle: &VirtualListScrollHandle,
        index: usize,
    ) {
        self.reset();
        scroll_handle.scroll_to_item(index, ScrollStrategy::Nearest);
    }

    pub(crate) fn handle_wheel(
        &mut self,
        scroll_handle: &VirtualListScrollHandle,
        event: &ScrollEvent,
        window: &Window,
    ) -> bool {
        self.track_touch_phase(event.touch_phase);
        let mut delta = vertical_wheel_delta(
            event.delta.pixel_delta(window.line_height()),
            event.modifiers.shift,
        );
        if delta.is_zero() {
            return false;
        }

        let touch_gesture = event.delta.precise() && self.touch_gesture_active;
        if !touch_gesture {
            let page = page_step(scroll_handle.as_ref().bounds().size.height);
            if !page.is_zero() {
                delta = delta.clamp(-page, page);
            }
        }

        let current_offset = scroll_handle.as_ref().offset();
        let current = current_offset.y;
        let max_offset = scroll_handle.as_ref().max_offset().y;
        let should_smooth = !touch_gesture;
        let target = if should_smooth {
            let target = self
                .animation
                .filter(|animation| animation.last_offset == current)
                .map(|animation| animation.target)
                .unwrap_or(current);
            (target + delta).clamp(-max_offset, px(0.0))
        } else {
            (current + delta).clamp(-max_offset, px(0.0))
        };

        if should_smooth {
            let last_frame_at = self
                .animation
                .filter(|animation| animation.last_offset == current)
                .map(|animation| animation.last_frame_at)
                .unwrap_or_else(Instant::now);
            self.animation = Some(ScrollAnimation {
                target,
                last_offset: current,
                last_frame_at,
            });
            self.schedule_next_frame(window);
        } else {
            scroll_handle.as_ref().set_offset(Point {
                x: current_offset.x,
                y: target,
            });
            self.animation = None;
        }

        true
    }

    pub(crate) fn advance(&mut self, scroll_handle: &VirtualListScrollHandle, window: &Window) {
        self.frame_scheduled = false;
        let Some(mut animation) = self.animation.take() else {
            return;
        };

        let current_offset = scroll_handle.as_ref().offset();
        let current = current_offset.y;
        if current != animation.last_offset {
            return;
        }

        let max_offset = scroll_handle.as_ref().max_offset().y;
        animation.target = animation.target.clamp(-max_offset, px(0.0));
        let now = Instant::now();
        let elapsed = now
            .saturating_duration_since(animation.last_frame_at)
            .as_secs_f32()
            .min(MAX_FRAME_SECONDS);
        animation.last_frame_at = now;
        let distance = animation.target - current;
        if distance.abs() <= px(SETTLE_DISTANCE) {
            scroll_handle.as_ref().set_offset(Point {
                x: current_offset.x,
                y: animation.target,
            });
            return;
        }

        let progress = 1.0 - (-elapsed / SMOOTHING_SECONDS).exp();
        let offset = current + distance * progress;
        animation.last_offset = offset;
        scroll_handle.as_ref().set_offset(Point {
            x: current_offset.x,
            y: offset,
        });
        self.animation = Some(animation);
        self.schedule_next_frame(window);
    }

    fn schedule_next_frame(&mut self, window: &Window) {
        if self.frame_scheduled {
            return;
        }
        self.frame_scheduled = true;
        window.on_next_frame(|window, _| window.refresh());
    }

    fn track_touch_phase(&mut self, touch_phase: TouchPhase) {
        match touch_phase {
            TouchPhase::Started => self.touch_gesture_active = true,
            TouchPhase::Ended | TouchPhase::Cancelled => self.touch_gesture_active = false,
            TouchPhase::Moved => {}
        }
    }
}

fn page_step(extent: Pixels) -> Pixels {
    (extent * PAGE_MIN_FRACTION)
        .max(extent - px(PAGE_MAX_OVERLAP))
        .max(px(0.0))
}

fn vertical_wheel_delta(delta: Point<Pixels>, shift: bool) -> Pixels {
    if shift && delta.y.is_zero() {
        delta.x
    } else {
        delta.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_scrolling_accepts_shift_converted_wheel() {
        let shifted = Point {
            x: px(-40.0),
            y: px(0.0),
        };
        assert_eq!(vertical_wheel_delta(shifted, true), px(-40.0));
        assert_eq!(vertical_wheel_delta(shifted, false), px(0.0));
    }

    #[test]
    fn only_touch_gestures_skip_smoothing() {
        let mut scroll = SmoothScroll::default();
        for (touch_phase, active) in [
            (TouchPhase::Moved, false),
            (TouchPhase::Started, true),
            (TouchPhase::Moved, true),
            (TouchPhase::Ended, false),
        ] {
            scroll.track_touch_phase(touch_phase);
            assert_eq!(scroll.touch_gesture_active, active);
        }
    }

    #[test]
    fn page_step_keeps_overlap() {
        assert_eq!(page_step(px(200.0)), px(175.0));
        assert_eq!(page_step(px(800.0)), px(760.0));
        assert_eq!(page_step(px(0.0)), px(0.0));
    }
}
