//! CLI-related methods for fixture display and tree printing.

use super::types::FixtureDefinition;
use super::FixtureDatabase;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

impl FixtureDatabase {
    /// Compute usage counts for all fixture definitions efficiently.
    fn compute_definition_usage_counts(&self) -> HashMap<(PathBuf, String), usize> {
        let mut counts: HashMap<(PathBuf, String), usize> = HashMap::new();

        // Initialize all definitions with 0 count
        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                counts.insert((def.file_path.clone(), fixture_name.clone()), 0);
            }
        }

        // Cache for resolved definitions
        let mut resolution_cache: HashMap<(PathBuf, String), Option<PathBuf>> = HashMap::new();

        // Pre-compute fixture definition lines per file
        let mut fixture_def_lines: HashMap<PathBuf, HashMap<usize, FixtureDefinition>> =
            HashMap::new();
        for entry in self.definitions.iter() {
            for def in entry.value().iter() {
                fixture_def_lines
                    .entry(def.file_path.clone())
                    .or_default()
                    .insert(def.line, def.clone());
            }
        }

        // Iterate all usages once
        for entry in self.usages.iter() {
            let file_path = entry.key();
            let usages = entry.value();
            let file_def_lines = fixture_def_lines.get(file_path);

            for usage in usages.iter() {
                let fixture_def_at_line = file_def_lines
                    .and_then(|lines| lines.get(&usage.line))
                    .cloned();

                let is_self_referencing = fixture_def_at_line
                    .as_ref()
                    .is_some_and(|def| def.name == usage.name);

                let resolved_def = if is_self_referencing {
                    self.find_closest_definition_excluding(
                        file_path,
                        &usage.name,
                        fixture_def_at_line.as_ref(),
                    )
                } else {
                    let cache_key = (file_path.clone(), usage.name.clone());
                    if let Some(cached) = resolution_cache.get(&cache_key) {
                        cached.as_ref().and_then(|def_path| {
                            self.definitions.get(&usage.name).and_then(|defs| {
                                defs.iter().find(|d| &d.file_path == def_path).cloned()
                            })
                        })
                    } else {
                        let def = self.find_closest_definition(file_path, &usage.name);
                        resolution_cache
                            .insert(cache_key, def.as_ref().map(|d| d.file_path.clone()));
                        def
                    }
                };

                if let Some(def) = resolved_def {
                    let key = (def.file_path.clone(), usage.name.clone());
                    *counts.entry(key).or_insert(0) += 1;
                }
            }
        }

        counts
    }

    /// Print fixtures as a tree structure
    pub fn print_fixtures_tree(&self, root_path: &Path, skip_unused: bool, only_unused: bool) {
        // Collect all files that define fixtures
        let mut file_fixtures: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();

        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            let definitions = entry.value();

            for def in definitions {
                file_fixtures
                    .entry(def.file_path.clone())
                    .or_default()
                    .insert(fixture_name.clone());
            }
        }

        let mut definition_usage_counts = self.compute_definition_usage_counts();

        let mut autouse_fixtures: HashSet<(PathBuf, String)> = HashSet::new();
        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                if def.autouse {
                    autouse_fixtures.insert((def.file_path.clone(), fixture_name.clone()));
                }
            }
        }

        // Remap editable install paths to virtual site-packages paths for display.
        // Only remap files that are outside the workspace (third-party editable installs).
        let mut editable_dirs: HashSet<PathBuf> = HashSet::new();
        {
            let installs = self.editable_install_roots.lock().unwrap();
            let workspace = self.workspace_root.lock().unwrap();
            let mut remapped: Vec<(PathBuf, PathBuf)> = Vec::new();

            for install in installs.iter() {
                // Skip editable installs that overlap with the workspace:
                // - source_root is inside workspace (in-workspace editable)
                // - workspace is inside source_root (project installed editable in its own venv)
                if let Some(ref ws) = *workspace {
                    if install.source_root.starts_with(ws) || ws.starts_with(&install.source_root) {
                        continue;
                    }
                }

                let keys_to_remap: Vec<PathBuf> = file_fixtures
                    .keys()
                    .filter(|p| p.starts_with(&install.source_root))
                    .cloned()
                    .collect();

                for original_path in keys_to_remap {
                    if let Ok(relative) = original_path.strip_prefix(&install.source_root) {
                        let virtual_path = install.site_packages.join(relative);
                        // Build label path from raw package name (dot-separated for namespace packages)
                        let parts: Vec<&str> = install.raw_package_name.split('.').collect();
                        if !parts.is_empty() {
                            let mut label_path = install.site_packages.clone();
                            for part in &parts {
                                label_path = label_path.join(part.replace('-', "_"));
                            }
                            editable_dirs.insert(label_path);
                        }
                        remapped.push((original_path, virtual_path));
                    }
                }
            }

            for (original, virtual_path) in &remapped {
                if let Some(fixtures) = file_fixtures.remove(original) {
                    file_fixtures.insert(virtual_path.clone(), fixtures);
                }
            }

            // Remap usage count keys to match virtual paths
            let mut remapped_counts: Vec<((PathBuf, String), (PathBuf, String))> = Vec::new();
            for (original, virtual_path) in &remapped {
                for key in definition_usage_counts.keys() {
                    if key.0 == *original {
                        remapped_counts.push((key.clone(), (virtual_path.clone(), key.1.clone())));
                    }
                }
            }
            for (old_key, new_key) in remapped_counts {
                if let Some(count) = definition_usage_counts.remove(&old_key) {
                    definition_usage_counts.insert(new_key, count);
                }
            }

            // Remap autouse fixture keys to match virtual paths
            let mut autouse_remapped: Vec<((PathBuf, String), (PathBuf, String))> = Vec::new();
            for (original, virtual_path) in &remapped {
                for key in autouse_fixtures.iter() {
                    if key.0 == *original {
                        autouse_remapped.push((key.clone(), (virtual_path.clone(), key.1.clone())));
                    }
                }
            }
            for (old_key, new_key) in autouse_remapped {
                autouse_fixtures.remove(&old_key);
                autouse_fixtures.insert(new_key);
            }
        }

        // Build a tree structure from paths
        let mut tree: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        let mut all_paths: BTreeSet<PathBuf> = BTreeSet::new();

        for file_path in file_fixtures.keys() {
            all_paths.insert(file_path.clone());

            let mut current = file_path.as_path();
            while let Some(parent) = current.parent() {
                if parent == root_path || parent.as_os_str().is_empty() {
                    break;
                }
                all_paths.insert(parent.to_path_buf());
                current = parent;
            }
        }

        for path in &all_paths {
            if let Some(parent) = path.parent() {
                if parent != root_path && !parent.as_os_str().is_empty() {
                    tree.entry(parent.to_path_buf())
                        .or_default()
                        .push(path.clone());
                }
            }
        }

        for children in tree.values_mut() {
            children.sort();
        }

        println!("Fixtures tree for: {}", root_path.display());
        println!();

        if file_fixtures.is_empty() {
            println!("No fixtures found in this directory.");
            return;
        }

        let mut top_level: Vec<PathBuf> = all_paths
            .iter()
            .filter(|p| {
                if let Some(parent) = p.parent() {
                    parent == root_path
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        top_level.sort();

        for (i, path) in top_level.iter().enumerate() {
            let is_last = i == top_level.len() - 1;
            self.print_tree_node(
                path,
                &file_fixtures,
                &tree,
                "",
                is_last,
                true,
                &definition_usage_counts,
                skip_unused,
                only_unused,
                &editable_dirs,
                &autouse_fixtures,
            );
        }
    }

    #[allow(clippy::too_many_arguments, clippy::only_used_in_recursion)]
    fn print_tree_node(
        &self,
        path: &Path,
        file_fixtures: &BTreeMap<PathBuf, BTreeSet<String>>,
        tree: &BTreeMap<PathBuf, Vec<PathBuf>>,
        prefix: &str,
        is_last: bool,
        is_root_level: bool,
        definition_usage_counts: &HashMap<(PathBuf, String), usize>,
        skip_unused: bool,
        only_unused: bool,
        editable_dirs: &HashSet<PathBuf>,
        autouse_fixtures: &HashSet<(PathBuf, String)>,
    ) {
        use colored::Colorize;

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

        let connector = if is_root_level {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        if file_fixtures.contains_key(path) {
            if let Some(fixtures) = file_fixtures.get(path) {
                let fixture_vec: Vec<_> = fixtures
                    .iter()
                    .filter(|fixture_name| {
                        let key = (path.to_path_buf(), (*fixture_name).clone());
                        let is_autouse = autouse_fixtures.contains(&key);
                        let usage_count = definition_usage_counts.get(&key).copied().unwrap_or(0);
                        if only_unused {
                            usage_count == 0 && !is_autouse
                        } else if skip_unused {
                            usage_count > 0 || is_autouse
                        } else {
                            true
                        }
                    })
                    .collect();

                if fixture_vec.is_empty() {
                    return;
                }

                let file_display = name.to_string().cyan().bold();
                println!(
                    "{}{}{} ({} fixtures)",
                    prefix,
                    connector,
                    file_display,
                    fixture_vec.len()
                );

                let new_prefix = if is_root_level {
                    "".to_string()
                } else {
                    format!("{}{}", prefix, if is_last { "    " } else { "│   " })
                };

                for (j, fixture_name) in fixture_vec.iter().enumerate() {
                    let is_last_fixture = j == fixture_vec.len() - 1;
                    let fixture_connector = if is_last_fixture {
                        "└── "
                    } else {
                        "├── "
                    };

                    let usage_count = definition_usage_counts
                        .get(&(path.to_path_buf(), (*fixture_name).clone()))
                        .copied()
                        .unwrap_or(0);

                    let is_autouse =
                        autouse_fixtures.contains(&(path.to_path_buf(), (*fixture_name).clone()));

                    let fixture_display = if is_autouse && usage_count == 0 {
                        fixture_name.to_string().cyan()
                    } else if usage_count == 0 {
                        fixture_name.to_string().dimmed()
                    } else {
                        fixture_name.to_string().green()
                    };

                    let usage_info = if is_autouse && usage_count == 0 {
                        "autouse=True".cyan().to_string()
                    } else if is_autouse {
                        format!(
                            "{}, {}",
                            if usage_count == 1 {
                                "used 1 time".yellow().to_string()
                            } else {
                                format!("used {} times", usage_count).yellow().to_string()
                            },
                            "autouse=True".cyan()
                        )
                    } else if usage_count == 0 {
                        "unused".dimmed().to_string()
                    } else if usage_count == 1 {
                        format!("{}", "used 1 time".yellow())
                    } else {
                        format!("{}", format!("used {} times", usage_count).yellow())
                    };

                    println!(
                        "{}{}{} ({})",
                        new_prefix, fixture_connector, fixture_display, usage_info
                    );
                }
            } else {
                println!("{}{}{}", prefix, connector, name);
            }
        } else if let Some(children) = tree.get(path) {
            let has_visible_children = children.iter().any(|child| {
                Self::has_visible_fixtures(
                    child,
                    file_fixtures,
                    tree,
                    definition_usage_counts,
                    skip_unused,
                    only_unused,
                    autouse_fixtures,
                )
            });

            if !has_visible_children {
                return;
            }

            let dir_label = if editable_dirs.contains(path) {
                format!("{}/ (editable install)", name)
            } else {
                format!("{}/", name)
            };
            let dir_display = dir_label.blue().bold();
            println!("{}{}{}", prefix, connector, dir_display);

            let new_prefix = if is_root_level {
                "".to_string()
            } else {
                format!("{}{}", prefix, if is_last { "    " } else { "│   " })
            };

            for (j, child) in children.iter().enumerate() {
                let is_last_child = j == children.len() - 1;
                self.print_tree_node(
                    child,
                    file_fixtures,
                    tree,
                    &new_prefix,
                    is_last_child,
                    false,
                    definition_usage_counts,
                    skip_unused,
                    only_unused,
                    editable_dirs,
                    autouse_fixtures,
                );
            }
        }
    }

    fn has_visible_fixtures(
        path: &Path,
        file_fixtures: &BTreeMap<PathBuf, BTreeSet<String>>,
        tree: &BTreeMap<PathBuf, Vec<PathBuf>>,
        definition_usage_counts: &HashMap<(PathBuf, String), usize>,
        skip_unused: bool,
        only_unused: bool,
        autouse_fixtures: &HashSet<(PathBuf, String)>,
    ) -> bool {
        if file_fixtures.contains_key(path) {
            if let Some(fixtures) = file_fixtures.get(path) {
                return fixtures.iter().any(|fixture_name| {
                    let key = (path.to_path_buf(), fixture_name.clone());
                    let is_autouse = autouse_fixtures.contains(&key);
                    let usage_count = definition_usage_counts.get(&key).copied().unwrap_or(0);
                    if only_unused {
                        usage_count == 0 && !is_autouse
                    } else if skip_unused {
                        usage_count > 0 || is_autouse
                    } else {
                        true
                    }
                });
            }
            false
        } else if let Some(children) = tree.get(path) {
            children.iter().any(|child| {
                Self::has_visible_fixtures(
                    child,
                    file_fixtures,
                    tree,
                    definition_usage_counts,
                    skip_unused,
                    only_unused,
                    autouse_fixtures,
                )
            })
        } else {
            false
        }
    }

    /// Get all unused fixtures (fixtures with zero usages).
    /// Returns a vector of (file_path, fixture_name) tuples sorted by path then name.
    /// Excludes third-party fixtures from site-packages.
    pub fn get_unused_fixtures(&self) -> Vec<(PathBuf, String)> {
        let definition_usage_counts = self.compute_definition_usage_counts();
        let mut unused: Vec<(PathBuf, String)> = Vec::new();

        for entry in self.definitions.iter() {
            let fixture_name = entry.key();
            for def in entry.value().iter() {
                // Skip third-party fixtures
                if def.is_third_party {
                    continue;
                }

                // Skip autouse fixtures (they're used implicitly)
                if def.autouse {
                    continue;
                }

                let usage_count = definition_usage_counts
                    .get(&(def.file_path.clone(), fixture_name.clone()))
                    .copied()
                    .unwrap_or(0);

                if usage_count == 0 {
                    unused.push((def.file_path.clone(), fixture_name.clone()));
                }
            }
        }

        // Sort by file path, then by fixture name for deterministic output
        unused.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        unused
    }
}
