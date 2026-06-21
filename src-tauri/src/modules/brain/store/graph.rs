//! Read-only knowledge-graph snapshot for the Brain Map (ADR-006). Nodes = project
//! hubs + files (capped per project by access frequency, so large repos stay
//! legible) + memory notes; edges = containment (project→file), imports
//! (file→file from `code_edges`), and anchors (note→file). Pure read over the
//! pinned WAL snapshot — never mutates. The radial layout is computed client-side;
//! this just supplies the typed nodes/edges + a degree for sizing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use super::{list_notes_with_conn, open_readonly_snapshot};

#[derive(Debug, Default, Serialize)]
pub struct BrainGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    /// `project` | `file` | `memory`
    pub kind: String,
    pub label: String,
    pub project_id: String,
    pub path: Option<String>,
    pub degree: u32,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub a: String,
    pub b: String,
    /// `contains` | `import` | `anchor`
    pub kind: String,
}

fn basename(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

/// Path-shaped anchor → bare indexed path (strip a leading `./` and a trailing
/// `:line`); `None` for symbol anchors (`mod::fn`, `path#sym`). Mirrors
/// `doctor::path_anchor` so anchor edges resolve against the same indexed paths.
fn anchor_path(a: &str) -> Option<String> {
    if a.contains("::") || a.contains('#') || !a.contains('/') {
        return None;
    }
    let a = a.strip_prefix("./").unwrap_or(a);
    let a = a.split(':').next().unwrap_or(a);
    (!a.is_empty()).then(|| a.to_string())
}

/// Build the whole-brain graph over `projects` (id, name), capping files per
/// project to `max_files` (most-accessed first). Import/anchor edges are kept only
/// between included nodes so the cap holds and the graph stays self-consistent.
pub fn graph_readonly(
    db_path: &Path,
    projects: &[(String, String)],
    max_files: usize,
) -> rusqlite::Result<BrainGraph> {
    let conn = open_readonly_snapshot(db_path)?;
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();

    for (pid, pname) in projects {
        let proj_id = format!("p:{pid}");
        nodes.push(GraphNode {
            id: proj_id.clone(),
            kind: "project".into(),
            label: pname.clone(),
            project_id: pid.clone(),
            path: None,
            degree: 0,
        });

        // Files — capped, most-accessed first (degree-light files drop off big repos).
        let mut included: HashSet<String> = HashSet::new();
        {
            let mut stmt = conn.prepare(
                "SELECT path FROM files WHERE project_id=?1 ORDER BY accessed_count DESC, path LIMIT ?2",
            )?;
            let rows =
                stmt.query_map(rusqlite::params![pid, max_files as i64], |r| r.get::<_, String>(0))?;
            for path in rows {
                let path = path?;
                let fid = format!("f:{pid}:{path}");
                nodes.push(GraphNode {
                    id: fid.clone(),
                    kind: "file".into(),
                    label: basename(&path),
                    project_id: pid.clone(),
                    path: Some(path.clone()),
                    degree: 0,
                });
                edges.push(GraphEdge { a: proj_id.clone(), b: fid, kind: "contains".into() });
                included.insert(path);
            }
        }

        // Memory notes + anchor edges into the included file set.
        for n in list_notes_with_conn(&conn, Some(pid)).unwrap_or_default() {
            let nid = format!("n:{pid}:{}", n.id);
            let label = if n.title.is_empty() { n.id.clone() } else { n.title.clone() };
            nodes.push(GraphNode {
                id: nid.clone(),
                kind: "memory".into(),
                label,
                project_id: pid.clone(),
                path: Some(n.path.clone()),
                degree: 0,
            });
            for a in &n.anchors {
                if let Some(ap) = anchor_path(a) {
                    if included.contains(&ap) {
                        edges.push(GraphEdge {
                            a: nid.clone(),
                            b: format!("f:{pid}:{ap}"),
                            kind: "anchor".into(),
                        });
                    }
                }
            }
        }

        // Import edges — only between included files, so the cap is self-consistent.
        {
            let mut stmt =
                conn.prepare("SELECT src_path,dst_path FROM code_edges WHERE project_id=?1")?;
            let rows =
                stmt.query_map([pid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (src, dst) = row?;
                if included.contains(&src) && included.contains(&dst) {
                    edges.push(GraphEdge {
                        a: format!("f:{pid}:{src}"),
                        b: format!("f:{pid}:{dst}"),
                        kind: "import".into(),
                    });
                }
            }
        }
    }

    // Degree (drives node sizing / ranking in the UI).
    let mut deg: HashMap<&str, u32> = HashMap::new();
    for e in &edges {
        *deg.entry(e.a.as_str()).or_default() += 1;
        *deg.entry(e.b.as_str()).or_default() += 1;
    }
    for n in &mut nodes {
        n.degree = deg.get(n.id.as_str()).copied().unwrap_or(0);
    }

    Ok(BrainGraph { nodes, edges })
}
