use std::time::Duration;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastTaskId(pub u64);

#[derive(Clone, Debug)]
struct Toast {
    id: u64,
    title: String,
    body: String,
    timeout_at: Option<Instant>,
    task_id: Option<ToastTaskId>,
    dismissed: bool,
    finished_task: bool,
}

impl Toast {
    fn is_active(&self, now: Instant) -> bool {
        !self.dismissed
            && !self.finished_task
            && self.timeout_at.is_none_or(|deadline| deadline > now)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToastView<'a> {
    id: u64,
    title: &'a str,
    body: &'a str,
}

impl<'a> ToastView<'a> {
    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn title(&self) -> &'a str {
        self.title
    }

    pub const fn body(&self) -> &'a str {
        self.body
    }
}

#[derive(Default)]
pub struct ToastManager {
    next_id: u64,
    toasts: Vec<Toast>,
}

impl ToastManager {
    const fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn push_timed(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        timeout: Duration,
    ) -> u64 {
        let id = self.alloc_id();
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body: body.into(),
            timeout_at: Some(Instant::now() + timeout),
            task_id: None,
            dismissed: false,
            finished_task: false,
        });
        id
    }

    pub fn push_task(&mut self, title: impl Into<String>, body: impl Into<String>) -> ToastTaskId {
        let id = self.alloc_id();
        let task_id = ToastTaskId(id);
        self.toasts.push(Toast {
            id,
            title: title.into(),
            body: body.into(),
            timeout_at: None,
            task_id: Some(task_id),
            dismissed: false,
            finished_task: false,
        });
        task_id
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(toast) = self.toasts.iter_mut().find(|toast| toast.id == id) {
            toast.dismissed = true;
        }
    }

    pub fn finish_task(&mut self, task_id: ToastTaskId) {
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                toast.finished_task = true;
            }
        }
    }

    pub fn update_task_body(&mut self, task_id: ToastTaskId, body: impl Into<String>) {
        let body = body.into();
        for toast in &mut self.toasts {
            if toast.task_id == Some(task_id) {
                toast.body.clone_from(&body);
            }
        }
    }

    pub fn prune(&mut self, now: Instant) {
        self.toasts.retain(|toast| toast.is_active(now));
    }

    pub fn active(&self, now: Instant) -> Vec<ToastView<'_>> {
        self.toasts
            .iter()
            .filter(|toast| toast.is_active(now))
            .map(|toast| ToastView {
                id: toast.id,
                title: &toast.title,
                body: &toast.body,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn timed_toast_expires() {
        let mut manager = ToastManager::default();
        manager.push_timed("settings", "updated", Duration::from_millis(10));
        assert_eq!(manager.active(Instant::now()).len(), 1);
        manager.prune(Instant::now() + Duration::from_millis(20));
        assert!(manager.active(Instant::now()).is_empty());
    }

    #[test]
    fn task_toast_finishes_independently() {
        let mut manager = ToastManager::default();
        let task = manager.push_task("cargo clean", "~/rust/bevy");
        assert_eq!(manager.active(Instant::now()).len(), 1);
        manager.finish_task(task);
        manager.prune(Instant::now());
        assert!(manager.active(Instant::now()).is_empty());
    }

    #[test]
    fn task_toast_body_can_be_updated() {
        let mut manager = ToastManager::default();
        let task = manager.push_task("startup git", "loading");
        manager.update_task_body(task, "2 remaining");
        let active = manager.active(Instant::now());
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].body(), "2 remaining");
    }
}
