//! 硬件能力提供者: 描述这台机器能做什么, 与具体游戏无关.
//!
//! 对每个游戏都可见, 由装配表在启动时登记一次; 贡献按同一份要求决定挂载.

use oddity_runtime::{
    Availability, Capability, CapabilityContext, CapabilityDescriptor, CapabilityId,
    CapabilityRevision, Scope,
};

/// CPU 调优: 核心绑定, 优先级与 I/O 优先级经 GameMode 下发.
pub const CPU_TUNING: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.tuning.cpu"),
    revision: CapabilityRevision(1),
    scope: Scope::NONE,
};

/// GPU 调优: 超频与功耗上限经 GameMode 下发, 需要可调优的显卡.
pub const GPU_TUNING: CapabilityDescriptor = CapabilityDescriptor {
    id: CapabilityId("oddity.tuning.gpu"),
    revision: CapabilityRevision(1),
    scope: Scope::NONE,
};

fn cpu_availability(context: &CapabilityContext) -> Availability {
    if context.hardware.game_mode {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

fn gpu_availability(context: &CapabilityContext) -> Availability {
    if context.hardware.game_mode && context.hardware.gpu {
        Availability::Available
    } else {
        Availability::Unsupported
    }
}

/// 硬件提供者声明的能力.
pub static CAPABILITIES: &[Capability] = &[
    Capability::new(CPU_TUNING, cpu_availability),
    Capability::new(GPU_TUNING, gpu_availability),
];

#[cfg(test)]
mod tests {
    use super::*;
    use oddity_runtime::HardwareFacts;

    fn context(hardware: HardwareFacts) -> CapabilityContext {
        CapabilityContext {
            game: "nte".into(),
            distribution: "cn".into(),
            account_id: None,
            session_running: false,
            launcher_present: true,
            hardware,
        }
    }

    fn state(facts: HardwareFacts, capability: &Capability) -> Availability {
        capability.availability(&context(facts))
    }

    #[test]
    fn cpu_and_gpu_tuning_follow_game_mode() {
        let off = HardwareFacts::default();
        let cpu = &CAPABILITIES[0];
        let gpu = &CAPABILITIES[1];
        assert_eq!(state(off, cpu), Availability::Unsupported);
        assert_eq!(state(off, gpu), Availability::Unsupported);

        // 开启 GameMode 后 CPU 调优可用, 没有可调优显卡时 GPU 仍不可用.
        let tuning = HardwareFacts {
            game_mode: true,
            gpu: false,
        };
        assert_eq!(state(tuning, cpu), Availability::Available);
        assert_eq!(state(tuning, gpu), Availability::Unsupported);

        let full = HardwareFacts {
            game_mode: true,
            gpu: true,
        };
        assert_eq!(state(full, cpu), Availability::Available);
        assert_eq!(state(full, gpu), Availability::Available);
    }

    #[test]
    fn hardware_capability_ids_are_well_formed_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for capability in CAPABILITIES {
            assert!(
                capability.descriptor.id.is_well_formed(),
                "{:?}",
                capability.descriptor.id
            );
            assert!(seen.insert(capability.descriptor.id.0), "重复能力");
        }
    }
}
