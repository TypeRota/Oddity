//! 下载域测试夹具: 假宿主, 事件记录与状态重置.
//!
//! 只在 `testing` feature 下编译; 宿主测试通过它注入设置快照与活动状态,
//! 并用全局锁串行化 (包级 static 是单实例语义).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use oddity_runtime::{ActivitySnapshot, EventSink, HostContext, SettingsSnapshot, Task, TaskJob};
use serde_json::Value;

/// 事件订阅回调.
type Listener = (&'static str, Box<dyn Fn(Value) + Send>);

/// 记录事件与任务的假宿主.
pub struct FakeHost {
    settings: Mutex<SettingsSnapshot>,
    activity: Mutex<ActivitySnapshot>,
    launcher_root: Mutex<Option<PathBuf>>,
    events: Mutex<Vec<(&'static str, Value)>>,
    listeners: Mutex<Vec<Listener>>,
    tasks: Mutex<HashMap<String, Task>>,
    cancelled: Mutex<Vec<String>>,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            settings: Mutex::new(SettingsSnapshot {
                game: "nte".into(),
                distribution: "cn".into(),
                language: "zh-CN".into(),
                reduce_motion: false,
                theme: "light".into(),
                install_root: None,
                preload_root: None,
                update: Default::default(),
            }),
            activity: Mutex::new(ActivitySnapshot::default()),
            launcher_root: Mutex::new(None),
            events: Mutex::new(Vec::new()),
            listeners: Mutex::new(Vec::new()),
            tasks: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(Vec::new()),
        }
    }
}

impl FakeHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_settings(&self, settings: SettingsSnapshot) {
        *self
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = settings;
    }

    pub fn settings(&self) -> SettingsSnapshot {
        self.settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set_activity(&self, activity: ActivitySnapshot) {
        *self
            .activity
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = activity;
    }

    pub fn set_launcher_root(&self, path: Option<PathBuf>) {
        *self
            .launcher_root
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = path;
    }

    pub fn events(&self) -> Vec<(String, Value)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(id, payload)| ((*id).to_string(), payload.clone()))
            .collect()
    }

    pub fn download_events(&self) -> Vec<Value> {
        self.events()
            .into_iter()
            .filter(|(id, _)| id == "game-download-state")
            .map(|(_, payload)| payload)
            .collect()
    }

    /// 订阅事件, 与宿主服务的测试订阅等价.
    pub fn subscribe(&self, id: &'static str, callback: impl Fn(Value) + Send + 'static) {
        self.listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((id, Box::new(callback)));
    }

    fn deliver_local(&self, id: &'static str, payload: &Value) {
        for (name, callback) in self
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
        {
            if *name == id {
                callback(payload.clone());
            }
        }
    }

    pub fn cancelled(&self) -> Vec<String> {
        self.cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

impl EventSink for FakeHost {
    fn deliver(
        &self,
        event: &'static oddity_contract::Event,
        payload: Value,
    ) -> Result<(), String> {
        self.deliver_local(event.id, &payload);
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((event.id, payload));
        Ok(())
    }
}

impl HostContext for FakeHost {
    fn settings(&self) -> Result<SettingsSnapshot, String> {
        Ok(self.settings())
    }

    fn activity(&self) -> Result<ActivitySnapshot, String> {
        Ok(self
            .activity
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone())
    }

    fn official_launcher_root(&self) -> Option<PathBuf> {
        self.launcher_root
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn emit(&self, id: &'static str, payload: Value) -> Result<(), String> {
        self.deliver_local(id, &payload);
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((id, payload));
        Ok(())
    }

    fn submit(&self, job: TaskJob) -> Task {
        let task = Task::new(Arc::new(|_| Ok(())));
        let handle = task.clone();
        self.tasks
            .lock()
            .unwrap()
            .insert(task.id().to_string(), handle);
        let runner = task.runner(job);
        std::thread::spawn(runner);
        task
    }

    fn cancel_task(&self, id: &str) -> bool {
        self.cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(id.to_string());
        if let Some(task) = self
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
        {
            task.cancel();
            true
        } else {
            false
        }
    }
}

/// 绑定假宿主并清空上一轮状态.
pub fn bind(host: &Arc<FakeHost>) {
    crate::update::state::bind_host(host.clone());
}

/// 重置绑定与下载状态; 测试之间必须串行调用.
pub fn reset() {
    crate::update::state::unbind_host();
    let manager = crate::update::state::manager();
    {
        let mut inner = manager
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *inner = crate::update::state::DownloadInner::default();
    }
    if let Ok(mut ledger) = manager.resource_ledger.lock() {
        *ledger = ();
    }
}

/// 串行化包级状态相关测试的全局锁.
pub fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|error| error.into_inner())
}
