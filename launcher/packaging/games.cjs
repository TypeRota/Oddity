// 外部启动的桌面动作按游戏生成. 新增游戏时在这里补一条, 与 nte-core 的
// GameId/profile 对应; 打包只负责动作名、菜单文案和默认启动参数.
const games = [
  {
    id: "nte",
    // 《异环》沿用旧动作标识, 已安装的桌面集成仍能按原标识启动.
    actions: [
      { key: "LaunchCn", name: "异环", region: "cn" },
      { key: "LaunchGlobal", name: "NTE", region: "global" },
    ],
  },
];

function launchActions() {
  return games.flatMap((game) =>
    game.actions.map((action) => ({
      key: action.key,
      name: action.name,
      exec: `oddity --launch --game ${game.id} --region ${action.region} --launch-mode background`,
    })),
  );
}

module.exports = { launchActions };
