//! Shared progress dialogs and cancellation controls for long-running jobs.

use super::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Identifies a registered job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobTicket(u64);

/// Describes how a registered job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The job has not been cancelled or stopped.
    Running,
    /// The job was cancelled.
    Cancelled,
    /// The job was stopped using its Stop button.
    Stopped,
}

struct Job {
    title: String,
    message: String,
    progress: au_runtime::Progress,
    cancel: au_runtime::CancelToken,
    started: Instant,
    can_stop: bool,
    outcome: Outcome,
}

struct JobView {
    ticket: JobTicket,
    title: String,
    message: String,
    fraction: f32,
    elapsed: Duration,
    can_stop: bool,
}

#[derive(Clone, Copy)]
enum JobAction {
    Cancel(JobTicket),
    Stop(JobTicket),
}

/// Tracks long-running jobs and displays their progress and cancellation controls.
pub struct ProgressCenter {
    next_ticket: u64,
    jobs: HashMap<JobTicket, Job>,
    order: Vec<JobTicket>,
    finished: HashMap<JobTicket, Outcome>,
}

impl Default for ProgressCenter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressCenter {
    /// Creates an empty progress center.
    pub fn new() -> Self {
        Self {
            next_ticket: 1,
            jobs: HashMap::new(),
            order: Vec::new(),
            finished: HashMap::new(),
        }
    }

    /// Registers a job handle and returns its ticket.
    pub fn register<T>(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        handle: &au_runtime::JobHandle<T>,
        can_stop: bool,
    ) -> JobTicket {
        self.register_parts(
            title,
            message,
            handle.progress.clone(),
            handle.cancel.clone(),
            can_stop,
        )
    }

    /// Registers progress and cancellation objects and returns the job's ticket.
    pub fn register_parts(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        progress: au_runtime::Progress,
        cancel: au_runtime::CancelToken,
        can_stop: bool,
    ) -> JobTicket {
        let ticket = JobTicket(self.next_ticket);
        self.next_ticket = self.next_ticket.wrapping_add(1);

        self.jobs.insert(
            ticket,
            Job {
                title: title.into(),
                message: message.into(),
                progress,
                cancel,
                started: Instant::now(),
                can_stop,
                outcome: Outcome::Running,
            },
        );
        self.order.push(ticket);
        ticket
    }

    /// Updates the message displayed for a registered job.
    #[allow(dead_code)] // API for export/import call sites (partner)
    pub fn set_message(&mut self, ticket: JobTicket, message: impl Into<String>) {
        if let Some(job) = self.jobs.get_mut(&ticket) {
            job.message = message.into();
        }
    }

    /// Removes a job and returns its outcome.
    ///
    /// A successfully completed job has the `Running` outcome because this
    /// API represents cancellation and stop state rather than completion.
    pub fn finish(&mut self, ticket: JobTicket) -> Outcome {
        let Some(job) = self.jobs.remove(&ticket) else {
            return self
                .finished
                .get(&ticket)
                .copied()
                .unwrap_or(Outcome::Running);
        };

        self.order.retain(|registered| *registered != ticket);
        let outcome = if job.outcome != Outcome::Running {
            job.outcome
        } else if job.cancel.is_cancelled() {
            Outcome::Cancelled
        } else {
            Outcome::Running
        };
        self.finished.insert(ticket, outcome);
        outcome
    }

    /// Returns the current or final outcome for a ticket, if known.
    #[allow(dead_code)] // API for export/import call sites (partner)
    pub fn outcome(&self, ticket: JobTicket) -> Option<Outcome> {
        if let Some(job) = self.jobs.get(&ticket) {
            if job.outcome != Outcome::Running {
                return Some(job.outcome);
            }
            return Some(if job.cancel.is_cancelled() {
                Outcome::Cancelled
            } else {
                Outcome::Running
            });
        }
        self.finished.get(&ticket).copied()
    }

    /// Cancels a registered job.
    pub fn cancel(&mut self, ticket: JobTicket) {
        if let Some(job) = self.jobs.get_mut(&ticket) {
            if job.outcome == Outcome::Running {
                job.outcome = Outcome::Cancelled;
            }
            job.cancel.cancel();
        }
    }

    /// Cancels all registered jobs.
    pub fn cancel_all(&mut self) {
        for job in self.jobs.values_mut() {
            if job.outcome == Outcome::Running {
                job.outcome = Outcome::Cancelled;
            }
            job.cancel.cancel();
        }
    }

    /// Returns whether there are no registered jobs.
    #[allow(dead_code)] // API for export/import call sites (partner)
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Returns the number of registered jobs.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Returns the title, progress fraction, and outcome of each registered job.
    pub fn snapshot(&self) -> Vec<(String, f32, Outcome)> {
        self.order
            .iter()
            .filter_map(|ticket| {
                let job = self.jobs.get(ticket)?;
                let outcome = if job.outcome != Outcome::Running {
                    job.outcome
                } else if job.cancel.is_cancelled() {
                    Outcome::Cancelled
                } else {
                    Outcome::Running
                };
                Some((job.title.clone(), job.progress.fraction(), outcome))
            })
            .collect()
    }

    /// Draws delayed progress dialogs and requests periodic repaints while jobs
    /// are registered.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.jobs.is_empty() {
            return;
        }

        ctx.request_repaint_after(Duration::from_millis(100));

        let now = Instant::now();
        let views: Vec<JobView> = self
            .order
            .iter()
            .filter_map(|ticket| {
                let job = self.jobs.get(ticket)?;
                let elapsed = now.saturating_duration_since(job.started);
                if elapsed < Duration::from_millis(500) {
                    return None;
                }

                let fraction = job.progress.fraction();
                Some(JobView {
                    ticket: *ticket,
                    title: job.title.clone(),
                    message: job.message.clone(),
                    fraction: if fraction.is_finite() {
                        fraction.clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                    elapsed,
                    can_stop: job.can_stop,
                })
            })
            .collect();

        let Some(first) = views.first() else {
            return;
        };

        let title = first.title.clone();
        let window_id = egui::Id::new(("progress-center", first.ticket.0));
        let mut actions = Vec::new();

        egui::Window::new(title)
            .id(window_id)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                for view in &views {
                    ui.push_id(view.ticket.0, |ui| {
                        if views.len() > 1 {
                            ui.label(&view.title);
                        }

                        ui.label(&view.message);
                        ui.add(egui::ProgressBar::new(view.fraction).show_percentage().desired_width(360.0));

                        let elapsed = view.elapsed.as_secs_f64();
                        let remaining = if view.fraction < 0.01 {
                            "--:--:--".to_owned()
                        } else {
                            Self::format_hms(
                                elapsed * (1.0 - f64::from(view.fraction))
                                    / f64::from(view.fraction),
                            )
                        };

                        ui.label(au_core::i18n::tr_format("Elapsed Time: %s", &[&Self::format_hms(elapsed)]));
                        ui.label(au_core::i18n::tr_format("Remaining Time: %s", &[&remaining]));

                        ui.horizontal(|ui| {
                            if ui.button(au_core::i18n::tr_ui("Cancel")).clicked() {
                                actions.push(JobAction::Cancel(view.ticket));
                            }
                            if view.can_stop && ui.button(au_core::i18n::tr_ui("Stop")).clicked() {
                                actions.push(JobAction::Stop(view.ticket));
                            }
                        });
                    });
                    ui.add_space(8.0);
                }
            });

        for action in actions {
            match action {
                JobAction::Cancel(ticket) => self.cancel(ticket),
                JobAction::Stop(ticket) => {
                    if let Some(job) = self.jobs.get_mut(&ticket) {
                        job.outcome = Outcome::Stopped;
                        job.cancel.cancel();
                    }
                }
            }
        }
    }

    fn format_hms(secs: f64) -> String {
        let total = if secs.is_finite() {
            secs.max(0.0).floor() as u64
        } else {
            0
        };
        let hours = total / 3_600;
        let minutes = (total % 3_600) / 60;
        let seconds = total % 60;
        format!("{hours}:{minutes:02}:{seconds:02}")
    }

    #[cfg(test)]
    fn backdate(&mut self, ticket: JobTicket, by: Duration) {
        if let Some(job) = self.jobs.get_mut(&ticket) {
            job.started = Instant::now().checked_sub(by).unwrap_or(job.started);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_runtime() -> au_runtime::Runtime {
        au_runtime::Runtime::new(1, 4)
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn cancel_bounds(output: &egui::FullOutput) -> Option<egui::Rect> {
        let update = output.platform_output.accesskit_update.as_ref()?;
        update.nodes.iter().find_map(|(_, node)| {
            if node.name() != Some("Cancel") {
                return None;
            }
            let bounds = node.bounds()?;
            Some(egui::Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            ))
        })
    }

    #[test]
    fn format_hms_formats_elapsed_and_remaining_time() {
        assert_eq!(ProgressCenter::format_hms(5.0), "0:00:05");
        assert_eq!(ProgressCenter::format_hms(3_661.9), "1:01:01");
        assert_eq!(ProgressCenter::format_hms(-1.0), "0:00:00");
    }

    #[test]
    fn register_cancel_and_finish_preserve_outcomes() {
        let runtime = test_runtime();
        let handle = runtime.submit(1, |ctx| {
            ctx.cancel.check()?;
            Ok(())
        });
        let mut center = ProgressCenter::new();
        let ticket = center.register("Test", "Working", &handle, false);

        assert_eq!(center.len(), 1);
        assert_eq!(center.outcome(ticket), Some(Outcome::Running));

        center.cancel(ticket);
        assert!(handle.cancel.is_cancelled());
        assert_eq!(center.outcome(ticket), Some(Outcome::Cancelled));
        assert_eq!(center.finish(ticket), Outcome::Cancelled);
        assert_eq!(center.outcome(ticket), Some(Outcome::Cancelled));
        assert!(center.is_empty());
    }

    #[test]
    fn accesskit_cancel_button_cancels_the_registered_job() {
        let runtime = test_runtime();
        let handle = runtime.submit(1, |ctx| {
            ctx.cancel.check()?;
            Ok(())
        });
        let cancel = handle.cancel.clone();
        let mut center = ProgressCenter::new();
        let ticket = center.register("Accessible test", "Working", &handle, false);
        center.backdate(ticket, Duration::from_secs(1));

        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let _ = ctx.run(input(Vec::new()), |context| center.show(context));
        let _ = ctx.run(input(Vec::new()), |context| center.show(context));
        // Read bounds after the window's sizing frames, when its position is final.
        let settled = ctx.run(input(Vec::new()), |context| center.show(context));
        let bounds = cancel_bounds(&settled)
            .expect("the accessible Cancel button should have bounds");
        let position = bounds.center();

        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(position)]),
            |context| center.show(context),
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]),
            |context| center.show(context),
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]),
            |context| center.show(context),
        );

        assert!(cancel.is_cancelled());
        assert_eq!(center.outcome(ticket), Some(Outcome::Cancelled));
    }
}
