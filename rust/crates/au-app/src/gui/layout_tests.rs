use super::*;
use std::collections::HashMap;

fn make_app() -> AudacityApp {
    let mut app = AudacityApp::default();
    app.toolbars = toolbars::ToolbarLayout::default_layout();
    app.editor.add_track(Track::new(
        "Track 1",
        au_core::AudioBuffer {
            sample_rate: 44_100,
            channels: vec![vec![0.0; 44_100]],
        },
    ));
    app
}

fn run_layout(
    app: &mut AudacityApp,
    size: egui::Vec2,
    scale: f32,
) -> (Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>, egui::Rect) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx.style_mut(|style| {
        for font in style.text_styles.values_mut() {
            font.size *= scale;
        }
    });

    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut last_nodes = None;

    for _ in 0..3 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| app.update_frame(ctx),
        );

        if let Some(update) = output.platform_output.accesskit_update {
            last_nodes = Some(update.nodes);
        }
    }

    (
        last_nodes.expect("headless app did not produce an AccessKit update"),
        screen,
    )
}

fn describe_node(id: egui::accesskit::NodeId, node: &egui::accesskit::Node) -> String {
    let name = node.name().unwrap_or("<unnamed>");
    match node.bounds() {
        Some(bounds) => format!(
            "node {id:?} name {name:?} bounds [{}, {}, {}, {}]",
            bounds.x0, bounds.y0, bounds.x1, bounds.y1
        ),
        None => format!("node {id:?} name {name:?} bounds <none>"),
    }
}

fn is_interactive(role: egui::accesskit::Role) -> bool {
    matches!(
        role,
        egui::accesskit::Role::Button
            | egui::accesskit::Role::CheckBox
            | egui::accesskit::Role::RadioButton
            | egui::accesskit::Role::ComboBox
    )
}

fn check_layout(
    nodes: &[(egui::accesskit::NodeId, egui::accesskit::Node)],
    screen: egui::Rect,
) -> Vec<String> {
    use egui::accesskit::Role;

    let mut problems = Vec::new();
    let top_level_menus = ["File", "Edit", "View", "Tracks", "Generate", "Effect", "Analyze"];

    for (id, node) in nodes {
        let name = node.name().unwrap_or("");
        let relevant_role = matches!(
            node.role(),
            Role::Button
                | Role::CheckBox
                | Role::ComboBox
                | Role::RadioButton
                | Role::MenuItem
                | Role::Label
        );

        if !relevant_role || name.trim().is_empty() {
            continue;
        }

        let Some(bounds) = node.bounds() else {
            continue;
        };
        let description = describe_node(*id, node);

        if !bounds.x0.is_finite()
            || !bounds.y0.is_finite()
            || !bounds.x1.is_finite()
            || !bounds.y1.is_finite()
        {
            problems.push(format!("{description}: bounds are not finite"));
            continue;
        }

        if bounds.x1 <= bounds.x0 || bounds.y1 <= bounds.y0 {
            problems.push(format!("{description}: bounds do not have positive size"));
        }

        if bounds.x0 < f64::from(screen.left()) - 0.5 || bounds.x0 >= f64::from(screen.right()) {
            problems.push(format!("{description}: left edge is outside the screen"));
        }

        if top_level_menus.contains(&name) {
            if bounds.x1 > f64::from(screen.right()) + 1.0 {
                problems.push(format!("{description}: menu extends past the right edge"));
            }
            if bounds.y1 > f64::from(screen.bottom()) + 1.0 {
                problems.push(format!("{description}: menu extends past the bottom edge"));
            }
        }
    }

    let mut parent_by_child = HashMap::new();
    for (parent_id, parent_node) in nodes {
        for child_id in parent_node.children() {
            parent_by_child.insert(*child_id, *parent_id);
        }
    }

    for first_index in 0..nodes.len() {
        let (first_id, first_node) = &nodes[first_index];
        if !is_interactive(first_node.role()) {
            continue;
        }
        let Some(first_parent) = parent_by_child.get(first_id) else {
            continue;
        };
        let Some(first_bounds) = first_node.bounds() else {
            continue;
        };

        for (second_id, second_node) in nodes.iter().skip(first_index + 1) {
            if !is_interactive(second_node.role())
                || parent_by_child.get(second_id) != Some(first_parent)
            {
                continue;
            }
            let Some(second_bounds) = second_node.bounds() else {
                continue;
            };

            let overlap_x =
                first_bounds.x1.min(second_bounds.x1) - first_bounds.x0.max(second_bounds.x0);
            let overlap_y =
                first_bounds.y1.min(second_bounds.y1) - first_bounds.y0.max(second_bounds.y0);

            if overlap_x > 1.0 && overlap_y > 1.0 {
                problems.push(format!(
                    "overlapping sibling controls: {} and {}",
                    describe_node(*first_id, first_node),
                    describe_node(*second_id, second_node)
                ));
            }
        }
    }

    problems
}

fn check_menus(nodes: &[(egui::accesskit::NodeId, egui::accesskit::Node)]) -> Vec<String> {
    let mut problems = Vec::new();

    let menu_x = |menu_name: &str| {
        nodes.iter().find_map(|(_, node)| {
            if node.name() == Some(menu_name) {
                node.bounds().map(|bounds| bounds.x0)
            } else {
                None
            }
        })
    };

    let file_x = menu_x("File");
    let view_x = menu_x("View");
    let effect_x = menu_x("Effect");

    if file_x.is_none() {
        problems.push("top-level menu \"File\" is missing".to_owned());
    }
    if view_x.is_none() {
        problems.push("top-level menu \"View\" is missing".to_owned());
    }
    if effect_x.is_none() {
        problems.push("top-level menu \"Effect\" is missing".to_owned());
    }
    if let (Some(file_x), Some(view_x), Some(effect_x)) = (file_x, view_x, effect_x) {
        if !(file_x < view_x && view_x < effect_x) {
            problems.push(format!(
                "top-level menus are out of order: File x={file_x}, View x={view_x}, Effect x={effect_x}"
            ));
        }
    }

    problems
}

#[test]
fn layout_is_clean_at_normal_text_size() {
    let mut app = make_app();
    let (nodes, screen) = run_layout(&mut app, egui::vec2(1280.0, 800.0), 1.0);
    let mut problems = check_layout(&nodes, screen);
    problems.extend(check_menus(&nodes));

    assert!(
        problems.is_empty(),
        "layout problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn layout_survives_long_translations() {
    let mut app = make_app();
    let (nodes, screen) = run_layout(&mut app, egui::vec2(1024.0, 700.0), 1.4);
    let mut problems = check_layout(&nodes, screen);
    problems.extend(check_menus(&nodes));

    assert!(
        problems.is_empty(),
        "layout problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn plugin_manager_dialog_fits() {
    let mut app = make_app();
    let size = egui::vec2(1024.0, 700.0);

    let (baseline_nodes, _) = run_layout(&mut app, size, 1.4);
    let baseline_ids: std::collections::HashSet<_> =
        baseline_nodes.iter().map(|(id, _)| *id).collect();

    app.plugin_manager = Some(Default::default());
    let (nodes, screen) = run_layout(&mut app, size, 1.4);
    let dialog_nodes: Vec<_> = nodes
        .into_iter()
        .filter(|(id, _)| !baseline_ids.contains(id))
        .collect();

    let problems = check_layout(&dialog_nodes, screen);
    assert!(
        problems.is_empty(),
        "layout problems:\n{}",
        problems.join("\n")
    );

    assert!(
        dialog_nodes.iter().any(|(_, node)| {
            node.name()
                .map(|name| name.contains("Rescan"))
                .unwrap_or(false)
        }),
        "plugin manager did not expose a node named \"Rescan\""
    );
}
