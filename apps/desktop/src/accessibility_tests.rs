use crate::{
    accessibility,
    project::{Document, Scene},
    session::Session,
    view::{Model, Painter},
};
#[test]
fn semantic_controls_match_visible_hit_targets_in_every_panel() {
    let mut painter = Painter::new();
    for dark_mode in [false, true] {
        for page in 0..6 {
            let scene = Scene::robotics_demo();
            let model = Model {
                dark_mode,
                workspace: true,
                inspector_page: page,
                document: Some(Document::new(scene.clone())),
                session: Some(Session::from_scene(&scene)),
                ..Default::default()
            };
            painter.paint(1100, 720, &model, None);
            let tree = accessibility::tree(&painter, &model);
            for hit in &painter.hits {
                let node = &tree
                    .nodes
                    .iter()
                    .find(|(id, _)| id.0 == hit.id as u64 + 2)
                    .expect("every control is accessible")
                    .1;
                assert!(!node.label().unwrap_or("").is_empty());
                assert!(node.supports_action(accesskit::Action::Focus));
                assert!(node.bounds().is_some());
            }
        }
    }
}
#[test]
fn splitters_expose_ranges_and_maximize_hides_panel_controls() {
    let scene = Scene::robotics_demo();
    let mut model = Model {
        workspace: true,
        document: Some(Document::new(scene.clone())),
        session: Some(Session::from_scene(&scene)),
        ..Default::default()
    };
    let mut painter = Painter::new();
    painter.dpi_scale = 1.5;
    painter.paint(1920, 1080, &model, None);
    let tree = accessibility::tree(&painter, &model);
    for id in [282, 283, 284] {
        let node = &tree
            .nodes
            .iter()
            .find(|(node, _)| node.0 == id + 2)
            .unwrap()
            .1;
        assert_eq!(node.role(), accesskit::Role::Slider);
        assert!(node.supports_action(accesskit::Action::SetValue));
        assert!(node.supports_action(accesskit::Action::Increment));
        assert!(node.numeric_value().unwrap() >= node.min_numeric_value().unwrap());
        assert!(node.numeric_value().unwrap() <= node.max_numeric_value().unwrap());
    }
    model.viewport_maximized = true;
    painter.paint(1920, 1080, &model, None);
    assert!(
        !painter
            .hits
            .iter()
            .any(|h| (282..=284).contains(&h.id) || h.id == 254)
    );
    assert!(painter.hits.iter().any(|h| h.id == 269));
}
#[test]
fn library_cards_and_appearance_tab_are_accessible_in_bounds_and_round_trip() {
    use crate::library::{self, Item};
    let mesh = pelagic::assets::Mesh::parse_obj(
        "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv 0 0 1\nf 1 2 3 4\nf 1 2 5\nf 2 3 5\nf 3 4 5\nf 4 1 5",
        "pyramid.obj",
    )
    .unwrap();
    let (scene, mesh_id, index) =
        library::import(&Scene::robotics_demo(), "pyramid.obj", mesh, None).unwrap();
    let scene = library::assign(&scene, Some(index), Item::Preset(4)).unwrap();
    let material = scene.entities[index].material.unwrap();
    let mut model = Model {
        workspace: true,
        document: Some(Document::new(scene.clone())),
        session: Some(Session::from_scene(&scene)),
        selected_library: Some(Item::Mesh(mesh_id)),
        ..Default::default()
    };
    let mut painter = Painter::new();
    for (filter, first) in [(1, library::MESH_CARD), (4, library::MATERIAL_CARD)] {
        model.content_filter = filter;
        for (w, h, dpi) in [(1100, 720, 1.0), (1920, 1080, 1.5), (1280, 640, 1.0)] {
            painter.dpi_scale = dpi;
            painter.paint(w, h, &model, None);
            let layout = painter.editor_layout.unwrap();
            let card = painter
                .hits
                .iter()
                .find(|h| h.id == first)
                .expect("library card visible on the first page");
            let [cx, cy, cw, ch] = card.rect;
            let [x, y, pw, ph] = layout.content;
            assert!(cx >= x && cy >= y && cx + cw <= x + pw && cy + ch <= y + ph + 0.5);
            for id in 417..=420 {
                assert!(
                    painter.accessible.iter().any(|a| a.id == id),
                    "library control {id} exposed"
                );
            }
            let tree = accessibility::tree(&painter, &model);
            let node = &tree
                .nodes
                .iter()
                .find(|(n, _)| n.0 == first as u64 + 2)
                .unwrap()
                .1;
            assert!(node.label().unwrap().starts_with("Select library item"));
            assert_eq!(
                node.role(),
                accesskit::Role::Button,
                "cards are invokable buttons"
            );
            // The card shows a rendered thumbnail, not a flat fill.
            let s = painter.ui_scale();
            let mut colours = std::collections::BTreeSet::new();
            for yy in 0..(30.0 * s) as usize {
                for xx in 0..(90.0 * s) as usize {
                    let px = (cx * s) as usize + 1 + xx;
                    let py = (cy * s) as usize + 1 + yy;
                    colours.insert(painter.pixels[py * painter.width + px]);
                }
            }
            assert!(colours.len() > 12, "thumbnail detail: {}", colours.len());
        }
    }
    // Appearance and material tabs for the selected actor.
    painter.dpi_scale = 1.0;
    model.inspector_page = 1;
    model.selected_entity = Some(index);
    model.tool_tab = 4;
    painter.paint(1100, 720, &model, None);
    assert!(painter.hits.iter().any(|h| h.id == 421));
    assert!(painter.hits.iter().any(|h| h.id == 422));
    let label = &painter
        .accessible
        .iter()
        .find(|a| a.id == 421)
        .unwrap()
        .label;
    assert_eq!(label, "Mesh: pyramid.obj");
    model.tool_tab = 5;
    painter.paint(1100, 720, &model, None);
    for id in 423..=427 {
        assert!(
            painter.hits.iter().any(|h| h.id == id),
            "material field {id}"
        );
    }
    let details = painter.editor_layout.unwrap().details;
    for hit in painter.hits.iter().filter(|h| (423..=427).contains(&h.id)) {
        assert!(hit.rect[1] + hit.rect[3] <= details[1] + details[3]);
    }
    // Commit a material field through the normal property path: one undo entry,
    // shared library value, save/reopen preserves handles.
    assert_eq!(crate::editor::field_value(&model, 426), "0.5");
    model.editing = Some((423, "12".into()));
    crate::editor::commit(&mut model).unwrap();
    let doc = model.document.as_ref().unwrap();
    assert_eq!(
        doc.scene.library.material(material).unwrap().base_color[0],
        12
    );
    assert!(doc.can_undo());
    model.editing = Some((426, "0".into()));
    assert!(
        crate::editor::commit(&mut model).is_err(),
        "roughness bound"
    );
    let dir = std::env::temp_dir().join(format!("pelagic-library-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("library.pelagic");
    model.document.as_mut().unwrap().save(&path).unwrap();
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.scene, model.document.as_ref().unwrap().scene);
    assert_eq!(reopened.scene.entities[index].asset, Some(mesh_id));
    std::fs::remove_dir_all(&dir).ok();
    crate::editor::history(&mut model, false).unwrap();
    assert_eq!(
        model
            .document
            .as_ref()
            .unwrap()
            .scene
            .library
            .material(material)
            .unwrap()
            .base_color,
        [236, 106, 28]
    );
}
#[test]
fn built_in_presets_do_not_crowd_the_all_assets_page() {
    let scene = Scene::robotics_demo();
    let mut model = Model {
        workspace: true,
        document: Some(Document::new(scene.clone())),
        session: Some(Session::from_scene(&scene)),
        ..Default::default()
    };
    let mut painter = Painter::new();
    painter.paint(1100, 720, &model, None);
    assert!(
        painter.hits.iter().any(|h| h.id == 91 + 256),
        "world data on page 1"
    );
    assert!(
        !painter
            .hits
            .iter()
            .any(|h| h.id >= crate::library::MATERIAL_CARD)
    );
    model.content_filter = 4;
    painter.paint(1100, 720, &model, None);
    assert!(
        painter
            .hits
            .iter()
            .any(|h| h.id == crate::library::MATERIAL_CARD)
    );
}
