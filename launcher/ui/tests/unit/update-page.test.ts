import { expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";

const host = vi.hoisted(() => {
  const state = {
    page: "nte-update",
    game: "idle",
    checking: false,
    bootstrap: {
      launcherRoot: "/games/nte/NTELauncher",
      installRoot: "/games/nte",
      settings: null,
    },
    download: null as Record<string, unknown> | null,
    resourceCatalog: null,
    resourcesSaving: false,
  };
  const form = {
    distribution: "cn" as string | null,
    installRoots: {} as Record<string, string>,
    reduceMotion: false,
  };
  return { state, form };
});

vi.mock("@host/api", async () => {
  const { computed, reactive } = await import("vue");
  const state = reactive(host.state);
  const form = reactive(host.form);
  const noop = () => undefined;
  const hostFacts = {
    game: computed(() => "nte"),
    distribution: computed(() => "cn"),
    channels: computed(() => [
      {
        id: "cn",
        label: "国服",
        default: true,
        active: true,
        installRootKey: "cn",
      },
      {
        id: "global",
        label: "国际服",
        default: false,
        active: false,
        installRootKey: "global",
      },
    ]),
  };
  return {
    hostFacts,
    update: {
      state,
      form,
      downloadBusy: () => false,
      downloadView: computed(() => ({
        title: "已是最新",
        progress: 0,
        downloadedText: "0 B",
        sizeText: "",
        splitText: "",
        stageText: "",
        autoText: "",
        detail: "",
      })),
      startGameDownload: noop,
      cancelGameDownload: noop,
      checkUpdate: noop,
      updateView: computed(() => ({
        rows: [],
        pending: 0,
        remaining: 0,
        showFiles: false,
        showHomeStatus: false,
        homeStatusKind: "",
        homeStatusText: "",
        kind: "idle",
        title: "已是最新",
        copy: "",
        icon: "i-check",
      })),
      formatSize: () => "0 B",
      diskBudget: computed(() => null),
      queueSettingsSave: noop,
      settingsPathsLocked: computed(() => false),
      runtimeEnvironmentBusy: computed(() => false),
      resourceCatalogMatches: computed(() => true),
      deletePreloadResources: noop,
      loadResourceCatalog: noop,
      downloadOptionalResource: noop,
      resourceDownloadsStarting: computed(() => false),
      removeOptionalResource: noop,
      gameSettings: () => ({
        distribution: "cn",
        installRoots: {},
        preloadRoot: null,
        updates: {},
        launchMode: "",
        gameAccountId: null,
        gameMode: {},
        activeEnvironment: null,
        stableEnvironment: null,
      }),
      factsFrom: () => ({
        activeGame: "nte",
        distribution: "cn",
        capabilities: [],
        channels: [],
      }),
      gameInstallRootHint: () => "",
    },
  };
});

import UpdatePage from "@packages/game-nte/ui/src/domains/update/UpdatePage.vue";
import { ntePageEntries } from "@packages/game-nte/ui";

it("update page is declared for nte and hidden for other games", () => {
  const entry = ntePageEntries.find((page) => page.id === "nte-update");
  expect(entry).toBeTruthy();
  expect(entry?.slot).toBe("drawer");
  expect(entry?.nav).toMatchObject({ label: "更新" });
  expect(
    entry?.visible({
      activeGame: "nte",
      distribution: "cn",
      capabilities: [],
      channels: [],
    }),
  ).toBe(true);
  expect(
    entry?.visible({
      activeGame: "fixture",
      distribution: "cn",
      capabilities: [],
      channels: [],
    }),
  ).toBe(false);
});

it("update page loads its game resource sections", () => {
  const wrapper = mount(UpdatePage);
  expect(wrapper.text()).toContain("游戏资源");
  expect(wrapper.text()).toContain("已是最新");
  wrapper.unmount();
});
