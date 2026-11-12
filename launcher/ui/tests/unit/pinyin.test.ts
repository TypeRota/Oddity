import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createPinyinEngine,
  isPinyinDict,
  normalizePinyin,
  type PinyinDict,
} from "../../src/core/pinyin";

const dict: PinyinDict = {
  n: [["嗯", 1]],
  ni: [
    ["你", 100],
    ["泥", 5],
  ],
  nian: [["年", 500]],
  nihao: [["你好", 50]],
  nh: [
    ["你好", 50],
    ["南海", 10],
  ],
  shi: [
    ["是", 1000],
    ["事", 200],
  ],
  xian: [["先", 80]],
  xiang: [["想", 400]],
  xianzai: [["现在", 60]],
};

function response(body: unknown) {
  return { ok: true, json: async () => body };
}

function failure(status: number) {
  return { ok: false, status, json: async () => ({}) };
}

describe("拼音规范化", () => {
  it("去掉大小写、空格、撇号和非字母", () => {
    expect(normalizePinyin(" Ni'Hao ")).toBe("nihao");
    expect(normalizePinyin("ni-hao")).toBe("nihao");
    expect(normalizePinyin("123")).toBe("");
    expect(normalizePinyin("lǜ")).toBe("lv");
  });
});

describe("候选匹配", () => {
  const engine = createPinyinEngine(dict);

  it("全拼、前缀和声母都能命中, 并给出消耗长度", () => {
    expect(engine.search("ni")[0]).toMatchObject({
      word: "你",
      matchedLength: 2,
    });
    expect(engine.search("nih")[0]).toMatchObject({
      word: "你好",
      matchedLength: 3,
    });
    expect(engine.search("niha")[0]).toMatchObject({
      word: "你好",
      matchedLength: 4,
    });
    expect(engine.search("nihao")[0]).toMatchObject({
      word: "你好",
      matchedLength: 5,
    });
    expect(engine.search("nh")[0]).toMatchObject({
      word: "你好",
      matchedLength: 2,
    });
  });

  it("同长度按词频排序, 长匹配优先", () => {
    expect(engine.search("shi").map((item) => item.word)).toEqual(["是", "事"]);
    expect(engine.search("xian").map((item) => item.word)).toEqual(["先"]);
    expect(engine.search("xianz").map((item) => item.word)).toEqual([
      "现在",
      "先",
    ]);
    const nihao = engine.search("nihao");
    expect(nihao.map((item) => item.word)).toEqual(["你好", "你", "泥", "嗯"]);
  });

  it("完整拼音精确命中时不展开更长音节", () => {
    // 「年」词频更高, 但用户只输入了 ni, 不能按空格直接上屏 nian.
    expect(engine.search("ni").map((item) => item.word)).not.toContain("年");
    expect(engine.search("ni")[0]).toMatchObject({ word: "你" });
    expect(engine.search("xian").map((item) => item.word)).not.toContain("想");
    expect(engine.search("xiang")[0]).toMatchObject({ word: "想" });
  });

  it("单个字母只查完全匹配, 不展开所有前缀", () => {
    expect(engine.search("n")[0]).toMatchObject({ word: "嗯" });
    expect(engine.search("x")).toEqual([]);
  });

  it("未知拼音返回空, 重复词保留最长匹配", () => {
    expect(engine.search("zzz")).toEqual([]);
    const deduped = createPinyinEngine({
      ni: [["你", 100]],
      nihao: [["你", 10]],
    });
    expect(deduped.search("nihao")[0]).toMatchObject({
      word: "你",
      matchedLength: 5,
      frequency: 10,
    });
  });
});

describe("词库校验", () => {
  it("接受合法词库", () => {
    expect(isPinyinDict(dict)).toBe(true);
  });

  it("拒绝非对象、非法键和坏条目", () => {
    expect(isPinyinDict(null)).toBe(false);
    expect(isPinyinDict([])).toBe(false);
    expect(isPinyinDict({ Ni: [["你", 1]] })).toBe(false);
    expect(isPinyinDict({ ni: "你" })).toBe(false);
    expect(isPinyinDict({ ni: [["你"]] })).toBe(false);
    expect(isPinyinDict({ ni: [["你", "1"]] })).toBe(false);
    expect(isPinyinDict({ ni: [["", 1]] })).toBe(false);
    expect(isPinyinDict({ ni: [[1, 1]] })).toBe(false);
    expect(isPinyinDict({ ni: [["你", Number.NaN]] })).toBe(false);
  });
});

describe("按需加载", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.unstubAllGlobals();
  });

  it("加载成功并缓存同一个引擎", async () => {
    const fetchMock = vi.fn().mockResolvedValue(response(dict));
    vi.stubGlobal("fetch", fetchMock);
    const { loadPinyinEngine: load } = await import("../../src/core/pinyin");
    const first = await load();
    const second = await load();
    expect(first).toBe(second);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith("./assets/pinyin-dict.json");
    expect(first.search("nihao")[0]?.word).toBe("你好");
  });

  it("HTTP 失败后清掉缓存并允许重试", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(failure(500))
      .mockResolvedValueOnce(response(dict));
    vi.stubGlobal("fetch", fetchMock);
    const { loadPinyinEngine: load } = await import("../../src/core/pinyin");
    await expect(load()).rejects.toThrow("HTTP 500");
    const engine = await load();
    expect(engine.search("ni")[0]?.word).toBe("你");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("词库格式不对时拒绝", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response({ bad: true })));
    const { loadPinyinEngine: load } = await import("../../src/core/pinyin");
    await expect(load()).rejects.toThrow("词库格式无效");
  });

  it("JSON 解析失败时拒绝", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => {
          throw new Error("bad json");
        },
      }),
    );
    const { loadPinyinEngine: load } = await import("../../src/core/pinyin");
    await expect(load()).rejects.toThrow("bad json");
  });
});
