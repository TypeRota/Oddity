/** 在玩家名后接欢迎语; 空名回退到通用文案. */
export function greeting(name: string): string {
  const trimmed = name.trim();
  return trimmed ? `${trimmed}, 欢迎回来` : "欢迎回来";
}
