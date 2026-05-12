use std::collections::HashMap;

use codeless_types::{Job, JobId, Repo, RepoId};
use parking_lot::RwLock;

/// In-memory store of repos and jobs. Lives only until stage 4 lands the
/// sqlx migration and runtime persistence — at that point this becomes
/// the test-double behind the same shape, not the production path.
/// `parking_lot::RwLock` over `tokio::sync::RwLock` because every method
/// here is non-async and the critical section is a single-map lookup;
/// pulling in an async lock would add `.await` syntax without buying
/// real cancellation safety.
#[derive(Default)]
pub struct MemoryStore {
    repos: RwLock<HashMap<RepoId, Repo>>,
    jobs: RwLock<HashMap<JobId, Job>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_repo(&self, repo: Repo) {
        self.repos.write().insert(repo.id, repo);
    }

    pub fn get_repo(&self, id: RepoId) -> Option<Repo> {
        self.repos.read().get(&id).cloned()
    }

    pub fn remove_repo(&self, id: RepoId) -> bool {
        self.repos.write().remove(&id).is_some()
    }

    pub fn list_repos(&self) -> Vec<Repo> {
        self.repos.read().values().cloned().collect()
    }

    pub fn insert_job(&self, job: Job) {
        self.jobs.write().insert(job.id, job);
    }

    pub fn get_job(&self, id: JobId) -> Option<Job> {
        self.jobs.read().get(&id).cloned()
    }

    pub fn update_job(&self, job: Job) -> bool {
        use std::collections::hash_map::Entry;
        let mut guard = self.jobs.write();
        match guard.entry(job.id) {
            Entry::Occupied(mut e) => {
                e.insert(job);
                true
            }
            Entry::Vacant(_) => false,
        }
    }

    pub fn list_jobs(&self, repo_filter: Option<RepoId>) -> Vec<Job> {
        self.jobs
            .read()
            .values()
            .filter(|j| match repo_filter {
                Some(r) => j.repo_id == r,
                None => true,
            })
            .cloned()
            .collect()
    }
}
