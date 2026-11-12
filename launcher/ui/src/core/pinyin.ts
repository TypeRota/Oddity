// 手柄屏幕键盘的拼音候选. 词库是离线生成的静态 JSON, 首次进入拼音模式时才加载.
export type PinyinEntry = [word: string, frequency: number];
export type PinyinDict = Record<string, PinyinEntry[]>;

export interface PinyinCandidate {
  word: string;
  /** 本次选词会消耗的拼音字母数. */
  matchedLength: number;
  frequency: number;
}

export interface PinyinEngine {
  search(input: string): PinyinCandidate[];
}

interface TrieNode {
  children: Map<string, TrieNode>;
  /** 经过该节点的词库键, 已按最高词频降序排列. */
  keys: string[];
}

const DICT_URL = "./assets/pinyin-dict.json";
const SEARCH_LIMIT = 200;
const PREFIX_KEY_LIMIT = 200;

export function normalizePinyin(input: string): string {
  return input
    .toLowerCase()
    .replace(/[üǖǘǚǜ]/g, "v")
    .replace(/[^a-z]/g, "");
}

/** 校验按需加载的词库, 避免坏数据把候选栏带崩. */
export function isPinyinDict(value: unknown): value is PinyinDict {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  for (const [key, entries] of Object.entries(value)) {
    if (!/^[a-z]+$/.test(key) || !Array.isArray(entries)) return false;
    for (const entry of entries) {
      if (
        !Array.isArray(entry) ||
        entry.length !== 2 ||
        typeof entry[0] !== "string" ||
        !entry[0] ||
        typeof entry[1] !== "number" ||
        !Number.isFinite(entry[1])
      )
        return false;
    }
  }
  return true;
}

export function createPinyinEngine(dict: PinyinDict): PinyinEngine {
  const root: TrieNode = { children: new Map(), keys: [] };
  const maxFrequency = new Map<string, number>();
  for (const [key, entries] of Object.entries(dict)) {
    let best = 0;
    for (const [, frequency] of entries) best = Math.max(best, frequency);
    maxFrequency.set(key, best);
    let node = root;
    node.keys.push(key);
    for (const letter of key) {
      let child = node.children.get(letter);
      if (!child)
        node.children.set(letter, (child = { children: new Map(), keys: [] }));
      node = child;
      node.keys.push(key);
    }
  }
  // 前缀命中可能很多, 先按最高词频排序, 只展开靠前的键.
  for (const node of walk(root))
    node.keys.sort((a, b) => maxFrequency.get(b)! - maxFrequency.get(a)!);

  function collectKeys(prefix: string): string[] {
    let node: TrieNode | undefined = root;
    for (const letter of prefix) {
      node = node.children.get(letter);
      if (!node) return [];
    }
    return node.keys;
  }

  function search(raw: string): PinyinCandidate[] {
    const input = normalizePinyin(raw);
    if (!input) return [];
    const found = new Map<string, PinyinCandidate>();
    const add = (word: string, frequency: number, matchedLength: number) => {
      const previous = found.get(word);
      if (
        !previous ||
        matchedLength > previous.matchedLength ||
        (matchedLength === previous.matchedLength &&
          frequency > previous.frequency)
      )
        found.set(word, { word, matchedLength, frequency });
    };
    const addEntries = (key: string, matchedLength: number) => {
      for (const [word, frequency] of dict[key] ?? [])
        add(word, frequency, matchedLength);
    };

    if (input.length === 1) {
      // 单个字母不展开所有前缀, 否则候选会多到没有意义.
      addEntries(input, 1);
    } else if (dict[input]?.length) {
      // 完整拼音已有精确键时不再展开更长音节, 否则输入 xian 会命中 xiang 的词,
      // 空格或确认键会把另一个音节的词当成当前输入上屏.
      addEntries(input, input.length);
    } else {
      for (const key of collectKeys(input).slice(0, PREFIX_KEY_LIMIT))
        addEntries(key, input.length);
    }
    // 较短前缀如果本身是完整音节也作为候选, 选词后可以继续输入.
    for (let length = input.length - 1; length >= 1; length--) {
      const key = input.slice(0, length);
      if (dict[key]) addEntries(key, length);
    }

    return [...found.values()]
      .sort(
        (a, b) =>
          b.matchedLength - a.matchedLength || b.frequency - a.frequency,
      )
      .slice(0, SEARCH_LIMIT);
  }

  return { search };
}

function* walk(node: TrieNode): Generator<TrieNode> {
  yield node;
  for (const child of node.children.values()) yield* walk(child);
}

let enginePromise: Promise<PinyinEngine> | null = null;

export function loadPinyinEngine(): Promise<PinyinEngine> {
  enginePromise ??= fetch(DICT_URL)
    .then((response) => {
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return response.json() as Promise<unknown>;
    })
    .then((data) => {
      if (!isPinyinDict(data)) throw new Error("词库格式无效");
      return createPinyinEngine(data);
    })
    .catch((error) => {
      // 失败后允许下次重新加载.
      enginePromise = null;
      throw error;
    });
  return enginePromise;
}
