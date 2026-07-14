use std::collections::{HashMap, HashSet, VecDeque};

use windows_sys_loader::Win32::System::LibraryLoader::GetProcAddress;

use chu_abi::ChuModDependsFunc;

use super::log::log_info;
use super::state::HMODULE;

pub struct PendingMod {
    pub file_name: String,
    pub full_path: String,
    pub handle: HMODULE,
    pub display_name: String,
    pub dependencies: Vec<String>,
}

unsafe impl Send for PendingMod {}

fn normalize_name(name: &str) -> String {
    let trimmed = name.trim();
    let without_ext = trimmed
        .strip_suffix(".dll")
        .or_else(|| trimmed.strip_suffix(".DLL"))
        .unwrap_or(trimmed);
    without_ext.to_ascii_lowercase()
}

pub fn mod_key(file_name: &str, display_name: &str) -> String {
    let display_key = normalize_name(display_name);
    if display_key.is_empty() {
        normalize_name(file_name)
    } else {
        display_key
    }
}

pub unsafe fn read_dependencies(handle: HMODULE) -> Vec<String> {
    let depends_ptr = GetProcAddress(handle, b"chumod_depends\0".as_ptr());
    let Some(depends_fn) = depends_ptr else {
        return Vec::new();
    };

    let depends_fn: ChuModDependsFunc = std::mem::transmute(depends_fn);
    let raw = depends_fn();
    if raw.is_null() {
        return Vec::new();
    }

    let text = std::ffi::CStr::from_ptr(raw).to_string_lossy();
    text.split(',')
        .map(normalize_name)
        .filter(|dep| !dep.is_empty())
        .collect()
}

pub fn sort_mods(mods: Vec<PendingMod>) -> Vec<PendingMod> {
    let mut by_key = HashMap::new();
    for (index, m) in mods.iter().enumerate() {
        let key = mod_key(&m.file_name, &m.display_name);
        by_key.insert(key, index);
        by_key.insert(normalize_name(&m.file_name), index);
    }

    let mut indegree = vec![0usize; mods.len()];
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); mods.len()];
    let mut seen_edges = HashSet::new();

    for (index, m) in mods.iter().enumerate() {
        for dep in &m.dependencies {
            let Some(&dep_index) = by_key.get(dep) else {
                log_info(&format!(
                    "dependency not loaded for {}: {}",
                    m.display_name, dep
                ));
                continue;
            };
            if dep_index == index {
                log_info(&format!("self dependency ignored: {}", m.display_name));
                continue;
            }
            if seen_edges.insert((dep_index, index)) {
                edges[dep_index].push(index);
                indegree[index] += 1;
            }
        }
    }

    let mut ready = VecDeque::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push_back(index);
        }
    }

    let mut ordered_indices = Vec::new();
    while let Some(index) = ready.pop_front() {
        ordered_indices.push(index);
        for &next in &edges[index] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                ready.push_back(next);
            }
        }
    }

    let ordered_set: HashSet<usize> = ordered_indices.iter().copied().collect();
    for (index, m) in mods.iter().enumerate() {
        if !ordered_set.contains(&index) {
            log_info(&format!(
                "dependency cycle detected, skip mod: {}",
                m.display_name
            ));
        }
    }

    let mut slots: Vec<Option<PendingMod>> = mods.into_iter().map(Some).collect();
    ordered_indices
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}
