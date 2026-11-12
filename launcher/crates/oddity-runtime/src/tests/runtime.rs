//! 长驻域缝: 注册表聚合, 默认门禁与启动前置检查.

use super::{TestHost, TEST_PACKAGE};
use crate::{
    GamePackage, LaunchError, LaunchMode, LaunchProvider, LaunchResult, PackageDescriptor,
    PackageGate, PackageRuntime, Registry, SessionTarget, SessionTicket,
};
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 记录生命周期的假长驻域.
struct FakeRuntime {
    busy: AtomicBool,
    blocking: bool,
    started: AtomicBool,
    stopped: AtomicBool,
    stop_handle: Mutex<Option<Arc<AtomicBool>>>,
}

impl FakeRuntime {
    const fn new(blocking: bool) -> Self {
        Self {
            busy: AtomicBool::new(false),
            blocking,
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            stop_handle: Mutex::new(None),
        }
    }
}

impl PackageRuntime for FakeRuntime {
    fn start(&self, _host: Arc<dyn crate::HostContext>, stop: Arc<AtomicBool>) {
        self.started.store(true, Ordering::Release);
        *self.stop_handle.lock().unwrap() = Some(stop);
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(stop) = self.stop_handle.lock().unwrap().take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    fn gate(&self) -> Result<Box<dyn PackageGate>, String> {
        Ok(Box::new(FakeGate {
            blocks: self.blocking,
        }))
    }
}

struct FakeGate {
    blocks: bool,
}

impl PackageGate for FakeGate {
    fn blocks_changes(&self) -> bool {
        self.blocks
    }
}

/// 带长驻域的测试包; 包与长驻域都必须是静态的.
struct RuntimePackage {
    id: &'static str,
    runtime: &'static FakeRuntime,
}

impl GamePackage for RuntimePackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: self.id,
            name: "运行时测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn runtime(&self) -> Option<&'static dyn PackageRuntime> {
        Some(self.runtime)
    }
}

/// 只实现 launch 的启动实现; check 走默认放行.
struct AllowProvider {
    launched: AtomicBool,
}

impl LaunchProvider for AllowProvider {
    fn launch(&self, _ticket: SessionTicket) -> Result<LaunchResult, LaunchError> {
        self.launched.store(true, Ordering::Release);
        Err(LaunchError::new(
            crate::LaunchFailureKind::Backend,
            "测试不拉起进程",
        ))
    }
}

fn target() -> SessionTarget {
    SessionTarget {
        game: "test".into(),
        distribution: "global".into(),
        mode: LaunchMode::Official,
    }
}

static OPEN_RUNTIME: FakeRuntime = FakeRuntime::new(false);
static BLOCKING_RUNTIME: FakeRuntime = FakeRuntime::new(true);

#[test]
fn package_without_a_runtime_reports_none() {
    let mut registry = Registry::new();
    registry.install(&TEST_PACKAGE);

    assert!(registry.runtime("test").is_none());
    assert_eq!(registry.runtimes().count(), 0);
    assert!(!registry.busy());
    assert!(registry.gates().unwrap().is_empty());
}

#[test]
fn runtimes_keep_assembly_order() {
    let first = RuntimePackage {
        id: "first",
        runtime: &OPEN_RUNTIME,
    };
    let second = RuntimePackage {
        id: "second",
        runtime: &BLOCKING_RUNTIME,
    };
    let mut registry = Registry::new();
    registry.install(&first);
    registry.install(&second);

    let runtimes: Vec<*const dyn PackageRuntime> = registry
        .runtimes()
        .map(|runtime| runtime as *const dyn PackageRuntime)
        .collect();
    assert_eq!(
        runtimes,
        [
            &OPEN_RUNTIME as *const FakeRuntime as *const dyn PackageRuntime,
            &BLOCKING_RUNTIME as *const FakeRuntime as *const dyn PackageRuntime,
        ]
    );
    assert!(registry.runtime("first").is_some());
    assert!(registry.runtime("second").is_some());
    assert!(registry.runtime("missing").is_none());
}

#[test]
fn an_open_gate_does_not_block_changes() {
    let package = RuntimePackage {
        id: "open",
        runtime: &OPEN_RUNTIME,
    };
    let mut registry = Registry::new();
    registry.install(&package);

    let gates = registry.gates().unwrap();
    assert_eq!(gates.len(), 1);
    assert!(!gates.iter().any(|gate| gate.blocks_changes()));
    assert!(Registry::new().gates().unwrap().is_empty());
}

#[test]
fn launch_check_defaults_to_allow() {
    let provider = AllowProvider {
        launched: AtomicBool::new(false),
    };
    assert_eq!(provider.check(&target()), Ok(()));
    assert!(!provider.launched.load(Ordering::Acquire));
}

#[test]
fn busy_gate_and_lifecycle_come_from_the_package() {
    let package = RuntimePackage {
        id: "blocking",
        runtime: &BLOCKING_RUNTIME,
    };
    let mut registry = Registry::new();
    registry.install(&package);

    assert!(!registry.busy());
    BLOCKING_RUNTIME.busy.store(true, Ordering::Release);
    assert!(registry.busy());
    let gates = registry.gates().unwrap();
    assert!(gates.iter().any(|gate| gate.blocks_changes()));
    drop(gates);

    let stop = Arc::new(AtomicBool::new(false));
    let host = TestHost::new();
    BLOCKING_RUNTIME.start(Arc::new(host), Arc::clone(&stop));
    assert!(BLOCKING_RUNTIME.started.load(Ordering::Acquire));
    BLOCKING_RUNTIME.stop();
    assert!(BLOCKING_RUNTIME.stopped.load(Ordering::Acquire));
    assert!(stop.load(Ordering::Acquire));

    BLOCKING_RUNTIME.busy.store(false, Ordering::Release);
    assert!(!registry.busy());

    // 默认无头入口: 不认领任何参数.
    assert_eq!(
        BLOCKING_RUNTIME.headless_entry(&[OsString::from("--ipc")]),
        None
    );
}
/// 只实现 start 的最小长驻域; 其余方法走默认实现.
struct MinimalRuntime;

static MINIMAL_RUNTIME: MinimalRuntime = MinimalRuntime;

impl PackageRuntime for MinimalRuntime {
    fn start(&self, _host: Arc<dyn crate::HostContext>, _stop: Arc<AtomicBool>) {}
}

struct MinimalPackage;

impl GamePackage for MinimalPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: "minimal",
            name: "最小包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn runtime(&self) -> Option<&'static dyn PackageRuntime> {
        Some(&MINIMAL_RUNTIME)
    }
}

#[test]
fn default_runtime_methods_are_inert() {
    let mut registry = Registry::new();
    registry.install(&MinimalPackage);
    let runtime = registry.runtime("minimal").expect("长驻域应已登记");

    assert!(!registry.busy());
    assert!(!registry.blocks_changes());
    let gates = registry.gates().unwrap();
    assert_eq!(gates.len(), 1);
    assert!(!gates[0].blocks_changes());

    runtime.stop();
    runtime.on_launch(&TestHost::new());
    assert_eq!(runtime.headless_entry(&[]), None);
    assert_eq!(runtime.headless_entry(&[OsString::from("--ipc")]), None);
}
