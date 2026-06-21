use std::sync::{Arc, Mutex};

use eframe::egui;
use polyglot_sql::ast_transforms::get_output_column_names;
use polyglot_sql::expressions::Expression;
use polyglot_sql::lineage::{LineageNode, lineage};
use polyglot_sql::{DialectType, parse_one};

use crate::QueryState;

pub(crate) fn detect_track_column(
    query: String,
    state: Arc<Mutex<QueryState>>,
    ctx: egui::Context,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // `compute` parses the SQL and recursively walks the resulting AST, which can be
        // deep for nested expressions (e.g. the `contains(lower(strip_accents(...)))` a
        // `:` match compiles to). The default 2 MiB worker-thread stack overflows on such
        // input in debug builds, so we give it generous headroom and a name (an unnamed
        // thread reports as `<unknown>` if it ever does overflow).
        std::thread::Builder::new()
            .name("lineage".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let result = compute(&query);
                apply(result, &state, &ctx);
            })
            .expect("failed to spawn lineage thread");
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            let result = compute(&query);
            apply(result, &state, &ctx);
        });
    }
}

fn compute(sql: &str) -> Option<usize> {
    let expr = parse_one(sql, DialectType::DuckDB).ok()?;
    let names = get_output_column_names(&expr);
    let mut found: Option<usize> = None;
    for (idx, name) in names.iter().enumerate() {
        let node = lineage(name, &expr, Some(DialectType::DuckDB), false).ok()?;
        if traces_to_track_id(&node) {
            if found.is_some() {
                return None;
            }
            found = Some(idx);
        }
    }
    found
}

fn traces_to_track_id(root: &LineageNode) -> bool {
    for node in root.walk() {
        if !node.downstream.is_empty() {
            continue;
        }
        let Expression::Table(table) = &node.source else {
            continue;
        };
        if !table.name.name.eq_ignore_ascii_case("track") {
            continue;
        }
        if let Expression::Column(col) = &node.expression
            && col.name.name.eq_ignore_ascii_case("id")
        {
            return true;
        }
    }
    false
}

fn apply(track_col: Option<usize>, state: &Mutex<QueryState>, ctx: &egui::Context) {
    let mut s = state.lock().unwrap();
    s.track_id_column = track_col;
    s.lineage_done = true;
    drop(s);
    ctx.request_repaint();
}
