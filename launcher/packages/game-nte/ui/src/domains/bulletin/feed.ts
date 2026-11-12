import { computed, ref, watch } from "vue";
import { hostFacts, invoke, notify } from "@host/api";
import type { BulletinFeed } from "./types";

const CACHE_KEY = "nte.bulletins.v3";
const CACHE_TTL_MS = 15 * 60 * 1000;

function readCache(distribution: string) {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(`${CACHE_KEY}.${distribution}`) || "",
    ) as { savedAt?: unknown; feed?: BulletinFeed };
    if (
      typeof parsed.savedAt !== "number" ||
      !parsed.feed ||
      !Array.isArray(parsed.feed.items) ||
      !Array.isArray(parsed.feed.errors)
    )
      return null;
    return { savedAt: parsed.savedAt, feed: parsed.feed };
  } catch {
    return null;
  }
}

function writeCache(feed: BulletinFeed, distribution: string) {
  try {
    window.localStorage.setItem(
      `${CACHE_KEY}.${distribution}`,
      JSON.stringify({ savedAt: Date.now(), feed }),
    );
  } catch {}
}

/**
 * 公告页的数据来源. 缓存按区服隔离, TTL 内直接复用; 新的请求会作废在途的旧请求,
 * 保证切换区服后不会把旧区服的公告写回界面或缓存.
 */
export function useBulletinFeed() {
  const feed = ref<BulletinFeed | null>(null);
  const loading = ref(false);
  let request = 0;

  const items = computed(() => feed.value?.items ?? []);
  const errors = computed(() => feed.value?.errors ?? []);

  async function fetchBulletins(silent = true, force = false) {
    const distribution = hostFacts.distribution.value;
    if (!distribution || loading.value) return;
    const cached = readCache(distribution);
    if (
      !force &&
      cached &&
      cached.feed.errors.length === 0 &&
      !feed.value?.errors.length &&
      Date.now() - cached.savedAt < CACHE_TTL_MS
    ) {
      feed.value = cached.feed;
      return;
    }
    const current = ++request;
    if (!feed.value && cached) feed.value = cached.feed;
    loading.value = true;
    try {
      const result = await invoke("fetch_bulletins", {
        distribution: distribution === "global" ? "global" : "cn",
      });
      if (current !== request) return;
      feed.value = result;
      if (result.errors.length === 0) writeCache(result, distribution);
      if (!silent && result.errors.length > 0 && result.items.length === 0) {
        notify("公告暂时无法更新，请稍后重试。", "error", 6000, {
          key: "bulletins",
          primary: {
            label: "重试",
            icon: "i-arrows-clockwise",
            run: () => void fetchBulletins(false, true),
          },
        });
      }
    } catch (error) {
      if (current !== request) return;
      if (!feed.value) {
        feed.value = {
          items: [],
          errors: [error instanceof Error ? error.message : String(error)],
        };
      }
      if (!silent) {
        notify(
          error instanceof Error ? error.message : String(error),
          "error",
          6000,
          {
            key: "bulletins",
            primary: {
              label: "重试",
              icon: "i-arrows-clockwise",
              run: () => void fetchBulletins(false, true),
            },
          },
        );
      }
    } finally {
      if (current === request) loading.value = false;
    }
  }

  // 启动数据就绪后才读取公告, 避免先用默认区服发一次请求;
  // 之后区服切换时清空旧区服的视图和缓存并重新读取.
  let started = false;
  watch(
    [() => hostFacts.ready.value, () => hostFacts.distribution.value],
    ([ready]) => {
      if (!ready) return;
      if (!started) {
        started = true;
        void fetchBulletins();
        return;
      }
      request++;
      feed.value = null;
      loading.value = false;
      void fetchBulletins();
    },
    { immediate: true },
  );

  return {
    items,
    errors,
    loading,
    refresh: () => fetchBulletins(false, true),
  };
}
