//! Read-only project monitoring, bounded session logs and process-tree resource samples.
use super::*;
use std::collections::{HashSet, VecDeque};
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Default)]
pub(super) struct Monitor {
    pub pids: Mutex<Vec<u32>>,
    logs: Mutex<VecDeque<Value>>,
    system: Mutex<System>,
    sampled: Mutex<Option<Instant>>,
}
impl Monitor {
    pub fn record(&self, event: &Value) {
        if !event["text"].is_string() { return; }
        let mut entry = event.clone();
        entry["time"] = json!(chrono::Local::now().to_rfc3339());
        let text: String = entry["text"].as_str().unwrap().chars().take(8192).collect();
        entry["text"] = json!(text);
        let mut logs = self.logs.lock().unwrap();
        logs.push_back(entry);
        while logs.len() > 300 { logs.pop_front(); }
    }
}
impl Backend {
    pub(super) fn monitor_snapshot(self: &Arc<Self>) -> Result<Value, Fault> {
        let mut system = self.monitor.system.lock().unwrap();
        let mut sampled = self.monitor.sampled.lock().unwrap();
        if sampled.map(|t| t.elapsed() >= Duration::from_secs(1)).unwrap_or(true) {
            system.refresh_processes(ProcessesToUpdate::All, true);
            system.refresh_memory();
            *sampled = Some(Instant::now());
        }
        let resources = |roots: Vec<u32>| {
            let mut ids: HashSet<Pid> = roots.into_iter().map(Pid::from_u32).collect();
            loop {
                let extra: Vec<Pid> = system.processes().iter().filter(|(_, p)| p.parent().map(|id| ids.contains(&id)).unwrap_or(false)).map(|(id, _)| *id).collect();
                let before = ids.len(); ids.extend(extra);
                if ids.len() == before { break; }
            }
            let mut cpu = 0.0_f32; let mut memory = 0_u64; let mut read = 0_u64; let mut written = 0_u64; let mut count = 0;
            for id in ids { if let Some(p) = system.process(id) { count += 1; cpu += p.cpu_usage(); memory += p.memory(); let disk = p.disk_usage(); read += disk.total_read_bytes; written += disk.total_written_bytes; } }
            json!({"cpuPercent":cpu / std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32,"memoryBytes":memory,"readBytes":read,"writtenBytes":written,"processCount":count})
        };
        let config = self.config();
        let mut projects = Vec::new();
        for p in config["projects"].as_array().into_iter().flatten() {
            let child = self.project_runtime(p["id"].as_str().unwrap_or(""))?;
            projects.push(json!({"id":p["id"],"name":p["name"],"slug":p["slug"],"enabled":p["enabled"],"busy":child.busy(),"job":child.job.lock().unwrap().clone(),"check":child.check.lock().unwrap().clone(),"vcs":child.vcs_job.lock().unwrap().clone(),"resources":resources(child.monitor.pids.lock().unwrap().clone()),"logs":child.monitor.logs.lock().unwrap().clone()}));
        }
        Ok(json!({"projects":projects,"scheduler":self.scheduler_status(),"backend":resources(vec![std::process::id()]),"hostMemoryUsed":system.used_memory(),"hostMemoryTotal":system.total_memory(),"sampledAt":chrono::Local::now().to_rfc3339()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_isolates_project_logs_and_bounds_history() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Backend::new(tmp.path().join("config.json"), tmp.path().into());
        root.save(json!({"projects":[{"id":"a","slug":"a","name":"A"},{"id":"b","slug":"b","name":"B"}]})).unwrap();
        let a = root.project_runtime("a").unwrap();
        for n in 0..305 { a.emit(json!({"type":"line","text":format!("line {n}")})); }
        let snapshot = root.handle("GET", "/api/monitor", json!({})).unwrap();
        assert_eq!(snapshot["projects"].as_array().unwrap().len(), 2);
        assert_eq!(snapshot["projects"][0]["logs"].as_array().unwrap().len(), 300);
        assert_eq!(snapshot["projects"][1]["logs"].as_array().unwrap().len(), 0);
        assert!(snapshot["backend"]["memoryBytes"].as_u64().unwrap() > 0);
        assert_eq!(snapshot["projects"][0]["resources"]["processCount"], 0);
    }
}
