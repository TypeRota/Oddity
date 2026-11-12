import { describe, expect, it } from "vitest";
import type { CapabilityRequirement } from "../../src/host/capability";
import {
  pageContributions,
  selectPageContributions,
} from "../../src/host/contributions";
import { factsFrom } from "../../src/host/facts";
import {
  selectHomeCards,
  type HomeCardContribution,
} from "../../src/host/home-cards";
import {
  selectNavigationItems,
  type PageContribution,
} from "../../src/host/pages";
import {
  selectSettingsSections,
  type SettingsSectionContribution,
} from "../../src/host/settings";
import { bootstrap } from "./fixtures";

const facts = factsFrom(
  bootstrap({
    capabilities: [
      {
        id: "oddity.test.ready",
        revision: 2,
        state: "available",
        code: null,
        message: null,
      },
      {
        id: "oddity.test.offline",
        revision: 1,
        state: "unavailable",
        code: "offline",
        message: "离线",
      },
      {
        id: "oddity.test.unsupported",
        revision: 1,
        state: "unsupported",
        code: null,
        message: null,
      },
    ],
  }),
);

const component = pageContributions[0].component;

function required(id: string, revision = 1): CapabilityRequirement {
  return { id, revision, necessity: "required" };
}

function optional(id: string, revision = 1): CapabilityRequirement {
  return { id, revision, necessity: "optional" };
}

function page(
  id: string,
  requires: readonly CapabilityRequirement[],
  visible: () => boolean = () => true,
): PageContribution {
  return {
    id,
    slot: "drawer",
    order: 1,
    nav: { label: id, icon: "test" },
    component,
    requires,
    visible,
  };
}

function card(
  id: string,
  requires: readonly CapabilityRequirement[],
  visible: () => boolean = () => true,
): HomeCardContribution {
  return { id, order: 1, component, requires, visible };
}

function section(
  id: string,
  requires: readonly CapabilityRequirement[],
  visible: () => boolean = () => true,
): SettingsSectionContribution {
  return {
    id,
    label: id,
    icon: "i-test",
    order: 1,
    component,
    entries: [],
    requires,
    visible,
  };
}

const pageContributionsUnderTest: PageContribution[] = [
  page("missing", [required("oddity.test.missing")]),
  page("unsupported", [required("oddity.test.unsupported")]),
  page("offline", [required("oddity.test.offline")]),
  page("revision", [required("oddity.test.ready", 1)]),
  page("available", [required("oddity.test.ready", 2)]),
  page("optional-visible", [optional("oddity.test.missing")]),
  page("optional-hidden", [optional("oddity.test.missing")], () => false),
];

const cardContributionsUnderTest: HomeCardContribution[] = [
  card("card-missing", [required("oddity.test.missing")]),
  card("card-offline", [required("oddity.test.offline")]),
  card("card-available", [required("oddity.test.ready", 2)]),
  card("card-optional-visible", [optional("oddity.test.missing")]),
  card("card-optional-hidden", [optional("oddity.test.missing")], () => false),
];

const settingsSectionContributionsUnderTest: SettingsSectionContribution[] = [
  section("section-missing", [required("oddity.test.missing")]),
  section("section-unsupported", [required("oddity.test.unsupported")]),
  section("section-available", [required("oddity.test.ready", 2)]),
  section("section-optional-visible", [optional("oddity.test.missing")]),
  section(
    "section-optional-hidden",
    [optional("oddity.test.missing")],
    () => false,
  ),
];

const mounted = ["available", "optional-visible"];

describe("贡献的 required 能力", () => {
  it("required 缺失或不可用时页面不挂载, 也不进入导航", () => {
    expect(
      selectPageContributions(pageContributionsUnderTest, "drawer", facts).map(
        (entry) => entry.id,
      ),
    ).toEqual(mounted);
    expect(
      selectNavigationItems(pageContributionsUnderTest, facts).map(
        (item) => item.id,
      ),
    ).toEqual(mounted);
  });

  it("required 缺失或不可用时首页卡片不挂载", () => {
    expect(
      selectHomeCards(facts, cardContributionsUnderTest).map(
        (entry) => entry.id,
      ),
    ).toEqual(["card-available", "card-optional-visible"]);
  });

  it("required 缺失或不可用时设置分区不挂载", () => {
    expect(
      selectSettingsSections(facts, settingsSectionContributionsUnderTest).map(
        (entry) => entry.id,
      ),
    ).toEqual(["section-available", "section-optional-visible"]);
  });

  it("optional 不参与挂载判定, 仍由 visible 决定", () => {
    expect(
      selectPageContributions(pageContributionsUnderTest, "drawer", facts).map(
        (entry) => entry.id,
      ),
    ).toContain("optional-visible");
    expect(
      selectPageContributions(pageContributionsUnderTest, "drawer", facts).map(
        (entry) => entry.id,
      ),
    ).not.toContain("optional-hidden");
  });
});
