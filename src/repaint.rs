use std::time::{Duration, Instant};
use winit::event_loop::ControlFlow;

/// egui's delayed requests must not turn into continuous scene redraws.
#[derive(Default)]
pub(crate) struct RepaintSchedule {
    deadline: Option<Instant>,
}
impl RepaintSchedule {
    pub fn after_frame(&mut self, now: Instant, delay: Duration) {
        self.deadline = now.checked_add(delay);
    }

    pub fn request(&mut self, deadline: Instant) {
        self.deadline = Some(
            self.deadline
                .map_or(deadline, |pending| pending.min(deadline)),
        );
    }

    pub fn next(&mut self, now: Instant, animated: bool) -> (bool, ControlFlow) {
        let due = self.deadline.is_some_and(|deadline| deadline <= now);
        if due {
            self.deadline = None;
        }
        let flow = self
            .deadline
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        (due || animated, flow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_ui_requests_wait_instead_of_continuously_rendering() {
        let now = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.after_frame(now, Duration::from_secs(1));
        for millis in [0, 1, 16, 100, 999] {
            assert_eq!(
                schedule.next(now + Duration::from_millis(millis), false),
                (false, ControlFlow::WaitUntil(now + Duration::from_secs(1)))
            );
        }
        assert_eq!(
            schedule.next(now + Duration::from_secs(1), false),
            (true, ControlFlow::Wait)
        );
        assert_eq!(
            schedule.next(now + Duration::from_secs(2), false),
            (false, ControlFlow::Wait)
        );
    }

    #[test]
    fn egui_delayed_repaint_does_not_mean_an_immediate_frame() {
        let context = egui::Context::default();
        let mut delay = Duration::ZERO;
        // Let egui's initial settling passes complete, then emulate a tooltip timer.
        for _ in 0..5 {
            let output = context.run(Default::default(), |ctx| {
                ctx.request_repaint_after(Duration::from_secs(1));
            });
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(context.has_requested_repaint());
        assert!(delay > Duration::ZERO && delay < Duration::MAX);
        let now = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.after_frame(now, delay);
        assert_eq!(
            schedule.next(now, false),
            (false, ControlFlow::WaitUntil(now + delay))
        );
        assert_eq!(schedule.next(now + delay, false), (true, ControlFlow::Wait));

        for _ in 0..5 {
            let output = context.run(Default::default(), |_| {});
            schedule.after_frame(
                now,
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            );
        }
        assert_eq!(schedule.next(now, false), (false, ControlFlow::Wait));
    }

    #[test]
    fn static_frames_settle_while_animation_can_keep_running() {
        let now = Instant::now();
        let mut schedule = RepaintSchedule::default();
        schedule.after_frame(now, Duration::MAX);
        assert_eq!(schedule.next(now, false), (false, ControlFlow::Wait));
        assert_eq!(schedule.next(now, true), (true, ControlFlow::Wait));
        assert_eq!(schedule.next(now, false), (false, ControlFlow::Wait));
        schedule.request(now + Duration::from_secs(1));
        schedule.request(now);
        assert_eq!(schedule.next(now, false), (true, ControlFlow::Wait));
    }
}
