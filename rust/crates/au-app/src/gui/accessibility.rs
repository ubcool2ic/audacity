use super::*;

impl AudacityApp {
    /// Gives a painted track its accessible name and role, and provides Audacity-style
    /// keyboard navigation. Call this immediately after allocating the track response.
    ///
    /// The application should add a `focused_track: Option<usize>` field to
    /// `AudacityApp`.
    pub(super) fn track_accessibility(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        index: usize,
        track_count: usize,
        name: &str,
        channels: usize,
        seconds: f64,
        selected: bool,
    ) {
        let channel_label = match channels {
            1 => "Mono".to_owned(),
            2 => "Stereo".to_owned(),
            count => format!("{count} channels"),
        };
        let label = format!(
            "{name} Audio Track, {channel_label}, {seconds:.2} seconds{}",
            if selected { ", Select On" } else { "" }
        );

        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                selected,
                &label,
            )
        });

        if self.focused_track == Some(index) && !response.has_focus() {
            response.request_focus();
        }
        if response.gained_focus() {
            self.focused_track = Some(index);
        }

        if response.has_focus() {
            let pressed_key = ui.input_mut(|input| {
                if input.modifiers != egui::Modifiers::NONE {
                    return None;
                }

                for key in [
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                    egui::Key::Home,
                    egui::Key::End,
                    egui::Key::Enter,
                ] {
                    if input.key_pressed(key) {
                        input.consume_key(egui::Modifiers::NONE, key);
                        return Some(key);
                    }
                }
                None
            });

            match pressed_key {
                Some(egui::Key::ArrowUp) if index > 0 => {
                    self.focused_track = Some(index - 1);
                }
                Some(egui::Key::ArrowDown) if index + 1 < track_count => {
                    self.focused_track = Some(index + 1);
                }
                // Tracks Behaviors > "Wrap around" (/GUI/CircularTrackNavigation).
                Some(egui::Key::ArrowUp) if self.settings.circular_track_navigation && track_count > 0 => {
                    self.focused_track = Some(track_count - 1);
                }
                Some(egui::Key::ArrowDown) if self.settings.circular_track_navigation && track_count > 0 => {
                    self.focused_track = Some(0);
                }
                Some(egui::Key::Home) if track_count > 0 => {
                    self.focused_track = Some(0);
                }
                Some(egui::Key::End) if track_count > 0 => {
                    self.focused_track = Some(track_count - 1);
                }
                Some(egui::Key::Enter) => {
                    self.selected_track = if self.selected_track == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                }
                _ => {}
            }
        }

        if response.has_focus() {
            let color = crate::theme::c32(self.theme.colour("TrackFocus1"));
            ui.painter().rect_stroke(
                response.rect.shrink(1.0),
                0.0,
                egui::Stroke::new(2.0, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one headless frame of the full app UI with AccessKit enabled.
    fn run_frame(
        app: &mut AudacityApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> egui::FullOutput {
        ctx.enable_accesskit();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        ctx.run(raw, |ctx| app.update_frame(ctx))
    }

    fn nodes(
        output: &egui::FullOutput,
    ) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit update should be present")
            .nodes
            .clone()
    }

    fn find_named<'a>(
        nodes: &'a [(egui::accesskit::NodeId, egui::accesskit::Node)],
        needle: &str,
    ) -> Vec<&'a egui::accesskit::Node> {
        nodes
            .iter()
            .filter_map(|(_, node)| {
                node.name()
                    .filter(|name| name.contains(needle))
                    .map(|_| node)
            })
            .collect()
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn test_app(tracks: usize) -> AudacityApp {
        let mut app = AudacityApp::default();
        for i in 0..tracks {
            app.editor.add_track(Track::new(
                format!("Track {}", i + 1),
                au_core::AudioBuffer {
                    sample_rate: 44_100,
                    channels: vec![vec![0.0f32; 44_100]],
                },
            ));
        }
        app
    }

    fn settle(app: &mut AudacityApp, ctx: &egui::Context) -> egui::FullOutput {
        let size = egui::vec2(1200.0, 800.0);
        run_frame(app, ctx, Vec::new(), size);
        run_frame(app, ctx, Vec::new(), size);
        run_frame(app, ctx, Vec::new(), size)
    }

    #[test]
    fn track_panel_exposes_named_tracks() {
        let mut app = test_app(2);
        let ctx = egui::Context::default();
        let output = settle(&mut app, &ctx);
        let tree_nodes = nodes(&output);

        assert!(!find_named(&tree_nodes, "Track 1 Audio Track, Mono, 1.00 seconds").is_empty());
        assert!(!find_named(&tree_nodes, "Track 2 Audio Track").is_empty());
    }

    #[test]
    fn toolbar_buttons_have_names_and_roles() {
        let mut app = test_app(1);
        let ctx = egui::Context::default();
        let output = settle(&mut app, &ctx);
        let tree_nodes = nodes(&output);

        for name in ["Play", "Stop"] {
            assert!(tree_nodes.iter().any(|(_, node)| {
                node.name().is_some_and(|node_name| node_name.contains(name))
                    && node.role() == egui::accesskit::Role::Button
            }));
        }
    }

    #[test]
    fn arrow_keys_move_track_focus_and_enter_selects() {
        let mut app = test_app(2);
        app.focused_track = Some(0);
        let ctx = egui::Context::default();
        let size = egui::vec2(1200.0, 800.0);
        let _ = settle(&mut app, &ctx);

        let _ = run_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)], size);
        let output = run_frame(&mut app, &ctx, Vec::new(), size);
        assert_eq!(app.focused_track, Some(1));

        let tree_nodes = nodes(&output);
        let track_two_id = tree_nodes
            .iter()
            .find(|(_, node)| {
                node.name()
                    .is_some_and(|name| name.contains("Track 2 Audio Track"))
            })
            .map(|(id, _)| *id)
            .expect("Track 2 should be in the AccessKit tree");
        let update = output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit update should be present");
        assert_eq!(update.focus, track_two_id);

        let _ = run_frame(&mut app, &ctx, vec![key(egui::Key::Enter)], size);
        let output = run_frame(&mut app, &ctx, Vec::new(), size);
        assert_eq!(app.selected_track, Some(1));
        assert!(!find_named(&nodes(&output), "Track 2 Audio Track, Mono, 1.00 seconds, Select On")
            .is_empty());
    }

    #[test]
    fn menu_bar_is_accessible() {
        let mut app = test_app(1);
        let ctx = egui::Context::default();
        let output = settle(&mut app, &ctx);
        let tree_nodes = nodes(&output);

        for name in ["File", "Edit", "View"] {
            assert!(!find_named(&tree_nodes, name).is_empty());
        }
    }
}
