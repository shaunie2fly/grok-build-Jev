//! Choose the codebase-memory project whose root covers `cwd`.
//!
//! Comparison uses `Path::components` only, so a trailing slash does not
//! change the root. This module does not touch the filesystem.

use std::cmp::Reverse;
use std::path::{Component, Path};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedProject {
    pub(crate) name: String,
    pub(crate) root_path: String,
    pub(crate) nodes: u64,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ProjectWire {
    name: String,
    root_path: String,
    #[serde(default)]
    nodes: u64,
    #[serde(default)]
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct ProjectsWire {
    projects: Vec<ProjectWire>,
}

pub(crate) fn select_project(cwd: &Path, projects: &[IndexedProject]) -> Option<String> {
    projects
        .iter()
        .filter(|project| covers(&project.root_path, cwd))
        .max_by(|left, right| rank(left).cmp(&rank(right)))
        .map(|project| project.name.clone())
}

pub(crate) fn parse_projects(raw: &str) -> Result<Vec<IndexedProject>, ()> {
    if let Ok(projects) = decode_projects(raw) {
        return Ok(projects);
    }
    let start = raw.find('{').ok_or(())?;
    let end = raw.rfind('}').ok_or(())?;
    let json = raw.get(start..=end).ok_or(())?;
    decode_projects(json)
}

fn decode_projects(raw: &str) -> Result<Vec<IndexedProject>, ()> {
    let listed: ProjectsWire = serde_json::from_str(raw).map_err(|_| ())?;
    Ok(listed
        .projects
        .into_iter()
        .map(|project| IndexedProject {
            name: project.name,
            root_path: project.root_path,
            nodes: project.nodes,
            size_bytes: project.size_bytes,
        })
        .collect())
}

fn covers(root_path: &str, cwd: &Path) -> bool {
    let root_components: Vec<Component<'_>> = Path::new(root_path).components().collect();
    let cwd_components: Vec<Component<'_>> = cwd.components().collect();
    cwd_components.starts_with(&root_components)
}

fn rank(project: &IndexedProject) -> (usize, u64, u64, Reverse<&str>) {
    (
        Path::new(&project.root_path).components().count(),
        project.nodes,
        project.size_bytes,
        Reverse(project.name.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::implementations::grok_build::code_graph::{
        IndexedProject, parse_projects, select_project,
    };

    fn project(name: &str, root_path: &str, nodes: u64, size_bytes: u64) -> IndexedProject {
        IndexedProject {
            name: name.to_string(),
            root_path: root_path.to_string(),
            nodes,
            size_bytes,
        }
    }

    #[test]
    fn select_project_prefers_the_longest_covering_root() {
        let projects = vec![
            project("wide", "/repo", 10, 10),
            project("nested", "/repo/crates", 5, 5),
        ];
        let cwd = Path::new("/repo/crates/xai-grok-tools");
        assert_eq!(select_project(cwd, &projects).as_deref(), Some("nested"));
    }

    #[test]
    fn select_project_breaks_equal_roots_by_node_count_then_name() {
        let projects = vec![
            project("zzz", "/repo", 10, 99),
            project("aaa", "/repo", 50, 1),
        ];
        assert_eq!(
            select_project(Path::new("/repo/src"), &projects).as_deref(),
            Some("aaa")
        );
    }

    #[test]
    fn select_project_returns_none_when_nothing_covers_cwd() {
        let projects = vec![project("other", "/somewhere", 1, 1)];
        assert_eq!(select_project(Path::new("/repo"), &projects), None);
    }

    #[test]
    fn parse_projects_reads_a_json_object_wrapped_in_log_text() {
        let raw = "level=info\n{\"projects\":[{\"name\":\"grok-build-Jev\",\"root_path\":\"/mnt/data/repos/grok-build-Jev\",\"nodes\":133395,\"size_bytes\":595066880}]}";
        let parsed = parse_projects(raw).unwrap();
        let project = parsed.first().expect("project");
        assert_eq!(project.name, "grok-build-Jev");
        assert_eq!(project.nodes, 133395);
        assert_eq!(project.size_bytes, 595066880);
    }

    #[test]
    fn parse_projects_defaults_missing_counts_to_zero() {
        let raw = r#"{"projects":[{"name":"n","root_path":"/repo"}]}"#;
        let parsed = parse_projects(raw).unwrap();
        let project = parsed.first().expect("project");
        assert_eq!(project.nodes, 0);
        assert_eq!(project.size_bytes, 0);
    }

    #[test]
    fn select_project_ignores_a_longer_root_that_does_not_cover_cwd() {
        let projects = vec![
            project("wide", "/repo", 1, 1),
            project("side", "/repo/other", 100, 100),
        ];
        assert_eq!(
            select_project(Path::new("/repo/crates"), &projects).as_deref(),
            Some("wide")
        );
    }

    #[test]
    fn select_project_breaks_equal_nodes_by_size_then_name() {
        let by_size = vec![project("b", "/repo", 10, 1), project("a", "/repo", 10, 50)];
        assert_eq!(
            select_project(Path::new("/repo"), &by_size).as_deref(),
            Some("a")
        );

        let by_name = vec![
            project("zzz", "/repo", 10, 5),
            project("aaa", "/repo", 10, 5),
        ];
        assert_eq!(
            select_project(Path::new("/repo"), &by_name).as_deref(),
            Some("aaa")
        );
    }

    #[test]
    fn select_project_treats_a_trailing_slash_as_the_same_root() {
        let projects = vec![
            project("slash", "/repo/", 1, 1),
            project("plain", "/repo", 9, 1),
        ];
        assert_eq!(
            select_project(Path::new("/repo/src"), &projects).as_deref(),
            Some("plain")
        );
    }
}
