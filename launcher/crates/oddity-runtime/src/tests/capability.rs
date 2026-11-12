//! 能力词汇与装配校验测试.

use crate::{
    Availability, Capability, CapabilityContext, CapabilityFailure, CapabilityId, CapabilityIssue,
    CapabilityRequirement, CapabilityRevision, GamePackage, Necessity, PackageDescriptor, Registry,
    Scope,
};

fn available(_: &CapabilityContext) -> Availability {
    Availability::Available
}

const ACCOUNT: crate::CapabilityDescriptor = crate::CapabilityDescriptor {
    id: CapabilityId("oddity.account.game"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT),
};

const BAD_ID: crate::CapabilityDescriptor = crate::CapabilityDescriptor {
    id: CapabilityId("account.game"),
    revision: CapabilityRevision(1),
    scope: Scope::GAME,
};

const NARROW: crate::CapabilityDescriptor = crate::CapabilityDescriptor {
    id: ACCOUNT.id,
    revision: CapabilityRevision(1),
    scope: Scope::GAME,
};

static ONE_PROVIDER: &[Capability] = &[Capability::new(ACCOUNT, available)];
static TWO_PROVIDERS: &[Capability] = &[
    Capability::new(ACCOUNT, available),
    Capability::new(ACCOUNT, available),
];
static BAD_PROVIDER: &[Capability] = &[Capability::new(BAD_ID, available)];
static NARROW_PROVIDER: &[Capability] = &[Capability::new(NARROW, available)];

static REQUIRED_ACCOUNT: &[CapabilityRequirement] = &[CapabilityRequirement::required(ACCOUNT)];
static OPTIONAL_ACCOUNT: &[CapabilityRequirement] = &[CapabilityRequirement::optional(ACCOUNT)];
static DUPLICATE_REQUIREMENTS: &[CapabilityRequirement] = &[
    CapabilityRequirement::required(ACCOUNT),
    CapabilityRequirement::required(ACCOUNT),
];
static REQUIRED_REVISION_TWO: &[CapabilityRequirement] = &[CapabilityRequirement {
    id: ACCOUNT.id,
    revision: CapabilityRevision(2),
    necessity: Necessity::Required,
    scope: ACCOUNT.scope,
}];
static REQUIRED_WIDE_SCOPE: &[CapabilityRequirement] = &[CapabilityRequirement {
    id: ACCOUNT.id,
    revision: ACCOUNT.revision,
    necessity: Necessity::Required,
    scope: Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT),
}];
static OPTIONAL_BAD_ID: &[CapabilityRequirement] = &[CapabilityRequirement {
    id: BAD_ID.id,
    revision: BAD_ID.revision,
    necessity: Necessity::Optional,
    scope: BAD_ID.scope,
}];

/// 只声明能力 sheet 与依赖的测试包.
struct CapabilityPackage {
    id: &'static str,
    capabilities: &'static [Capability],
    requires: &'static [CapabilityRequirement],
}

impl GamePackage for CapabilityPackage {
    fn descriptor(&self) -> PackageDescriptor {
        PackageDescriptor {
            id: self.id,
            name: "能力测试包",
        }
    }

    fn register(&self, _registry: &mut Registry) {}

    fn capabilities(&self) -> &'static [Capability] {
        self.capabilities
    }

    fn requires(&self) -> &'static [CapabilityRequirement] {
        self.requires
    }
}

fn registry(
    capabilities: &'static [Capability],
    requires: &'static [CapabilityRequirement],
) -> Registry {
    let mut registry = Registry::new();
    registry.install(&CapabilityPackage {
        id: "test",
        capabilities,
        requires,
    });
    registry
}

fn context() -> CapabilityContext {
    CapabilityContext {
        game: "test".into(),
        distribution: "cn".into(),
        account_id: None,
        session_running: false,
        launcher_present: false,
    }
}

#[test]
fn scope_bits_union_and_containment() {
    assert_eq!(Scope::NONE.bits(), 0);
    let combined = Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT);
    assert_eq!(
        combined.bits(),
        Scope::GAME.bits() | Scope::DISTRIBUTION.bits() | Scope::ACCOUNT.bits()
    );
    assert!(combined.contains(Scope::GAME));
    assert!(combined.contains(Scope::ACCOUNT));
    assert!(combined.contains(Scope::NONE));
    assert!(!combined.contains(Scope::SESSION));
    assert!(!Scope::GAME.contains(Scope::DISTRIBUTION));
    assert_eq!(combined.to_string(), "game|distribution|account");
    assert_eq!(Scope::NONE.to_string(), "none");
    assert_eq!(CapabilityRevision(1).to_string(), "1");
}

#[test]
fn capability_id_rejects_malformed_names() {
    assert!(CapabilityId("oddity.account.game").is_well_formed());
    assert_eq!(
        CapabilityId("oddity.account.game").to_string(),
        "oddity.account.game"
    );
    for id in [
        "account.game",
        "oddity.account",
        "oddity..game",
        "oddity.Account.Game",
        "oddity.account.game.extra",
        "",
        ".oddity.account.game",
    ] {
        assert!(!CapabilityId(id).is_well_formed(), "{id}");
    }
}

#[test]
fn availability_maps_to_stable_codes() {
    assert_eq!(Availability::Available.into_gate("不支持"), Ok(()));

    let unsupported = Availability::Unsupported.into_gate("不支持").unwrap_err();
    assert_eq!(
        unsupported,
        CapabilityFailure {
            code: "unsupported",
            reason: None,
            message: "不支持",
        }
    );

    let unavailable = Availability::Unavailable {
        code: "launcher_missing",
        message: "请先安装官方组件和游戏资源.",
    }
    .into_gate("不支持")
    .unwrap_err();
    assert_eq!(
        unavailable,
        CapabilityFailure {
            code: "capability_unavailable",
            reason: Some("launcher_missing"),
            message: "请先安装官方组件和游戏资源.",
        }
    );

    let capability = Capability::new(ACCOUNT, available);
    assert_eq!(capability.descriptor, ACCOUNT);
    assert_eq!(capability.availability(&context()), Availability::Available);
}

#[test]
fn missing_required_capability_is_reported() {
    let registry = registry(&[], REQUIRED_ACCOUNT);
    assert_eq!(
        registry.validate_capabilities().unwrap_err(),
        [CapabilityIssue::MissingProvider {
            game: "test",
            consumer: "test",
            capability: "oddity.account.game",
        }]
    );
    assert!(registry.validate_capabilities().unwrap_err()[0]
        .to_string()
        .contains("缺少必需能力 oddity.account.game"));
}

#[test]
fn optional_capability_missing_is_not_an_error() {
    assert_eq!(
        registry(&[], OPTIONAL_ACCOUNT).validate_capabilities(),
        Ok(())
    );
}

#[test]
fn revision_mismatch_is_reported() {
    assert_eq!(
        registry(ONE_PROVIDER, REQUIRED_REVISION_TWO)
            .validate_capabilities()
            .unwrap_err(),
        [CapabilityIssue::RevisionMismatch {
            game: "test",
            consumer: "test",
            capability: "oddity.account.game",
            required: 2,
            provided: 1,
        }]
    );
}

#[test]
fn scope_mismatch_is_reported() {
    assert_eq!(
        registry(NARROW_PROVIDER, REQUIRED_WIDE_SCOPE)
            .validate_capabilities()
            .unwrap_err(),
        [CapabilityIssue::ScopeMismatch {
            game: "test",
            consumer: "test",
            capability: "oddity.account.game",
            required: Scope::GAME.union(Scope::DISTRIBUTION).union(Scope::ACCOUNT),
            provided: Scope::GAME,
        }]
    );
}

#[test]
fn duplicate_provider_and_requirement_are_reported() {
    assert_eq!(
        registry(TWO_PROVIDERS, DUPLICATE_REQUIREMENTS)
            .validate_capabilities()
            .unwrap_err(),
        [
            CapabilityIssue::DuplicateProvider {
                game: "test",
                capability: "oddity.account.game",
            },
            CapabilityIssue::DuplicateRequirement {
                game: "test",
                consumer: "test",
                capability: "oddity.account.game",
            },
        ]
    );
}

#[test]
fn invalid_capability_id_is_reported() {
    assert_eq!(
        registry(BAD_PROVIDER, &[])
            .validate_capabilities()
            .unwrap_err(),
        [CapabilityIssue::InvalidId {
            capability: "account.game",
        }]
    );
    assert_eq!(
        registry(&[], OPTIONAL_BAD_ID)
            .validate_capabilities()
            .unwrap_err(),
        [CapabilityIssue::InvalidId {
            capability: "account.game",
        }]
    );
}

#[test]
fn valid_capability_sheet_passes() {
    let registry = registry(ONE_PROVIDER, REQUIRED_ACCOUNT);
    assert_eq!(registry.validate_capabilities(), Ok(()));
    let sheet = registry.capabilities("test").expect("能力 sheet 应已装配");
    assert_eq!(sheet.len(), 1);
    assert_eq!(sheet[0].descriptor, ACCOUNT);
    assert_eq!(
        registry
            .requires("test")
            .map(<[CapabilityRequirement]>::len),
        Some(1)
    );
    assert!(registry.capabilities("missing").is_none());
}
