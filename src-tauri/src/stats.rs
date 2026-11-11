use std::collections::HashMap;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub struct Stats {
    system: Option<System>,
    cores: f32,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            system: System::new().ok(),
            cores: std::thread::available_parallelism()
                .map(|n| n.get() as f32)
                .unwrap_or(1.0),
        }
    }

    pub fn sample(&mut self, session_ids: &[String]) -> HashMap<String, (u64, f32)> {
        let mut usage: HashMap<String, (u64, f32)> = HashMap::new();
        let Some(system) = self.system.as_mut() else {
            return usage;
        };
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );

        let needles: Vec<(String, &String)> = session_ids
            .iter()
            .map(|id| (format!("profiles\\{}", id.to_lowercase()), id))
            .collect();

        for process in system.processes().values() {
            if !process
                .name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with("msedgewebview2")
            {
                continue;
            }
            let cmd = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some((_, id)) = needles.iter().find(|(needle, _)| cmd.contains(needle.as_str())) {
                let entry = usage.entry((*id).clone()).or_default();
                entry.0 += process.memory();
                entry.1 += process.cpu_usage() / self.cores;
            }
        }
        usage
    }
}
