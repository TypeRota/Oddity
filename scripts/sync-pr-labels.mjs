const pathRules = {
  "area: ui": [
    /^launcher\/(ui|tools\/ui-registry-gen)\//,
    /^launcher\/packages\/[^/]+\/ui\//,
  ],
  "area: backend": [/^launcher\/crates\/oddity-backend\//],
  "area: cli": [/^launcher\/crates\/oddity-cli\//],
  "area: contract": [
    /^launcher\/(crates\/oddity-contract|tools\/contract-gen)\//,
    /^launcher\/CONTRACTS\.md$/,
    /^launcher\/ui\/(electron\/commands|src\/core\/generated\/command-ids|src\/types\/generated\/commands)\.ts$/,
  ],
  "area: host": [/^launcher\/crates\/oddity-host\//],
  "area: platform": [/^launcher\/crates\/oddity-platform\//],
  "area: recovery": [/^launcher\/crates\/oddity-recovery\//],
  "area: runtime": [
    /^launcher\/crates\/oddity-runtime\//,
    /^launcher\/packages\/game-fixture\/backend\//,
  ],
  "area: nte": [/^launcher\/packages\/game-nte\//, /^launcher\/helpers\/nte-/],
  "area: arknights": [/^launcher\/packages\/game-arknights\//],
  "area: azurpromilia": [/^launcher\/packages\/game-azurpromilia\//],
  "area: hsr": [/^launcher\/packages\/game-hsr\//],
  "area: packaging": [
    /^(launcher\/packaging|packaging|nix)\//,
    /^launcher\/scripts\/install-desktop\.sh$/,
    /^launcher\/ui\/(electron-builder\.cjs|electron\/after-pack\.cjs)$/,
    /^scripts\/(build-arch-package|check-(gentoo-upgrade|packaging|release-(assets|version))|release-version|render-(ebuild|pkgbuild)|stage-release|write-checksums)\./,
    /^flake\.(nix|lock)$/,
  ],
  "area: updater": [/^launcher\/packages\/[^/]+\/backend\/src\/update\//],
  "area: proton": [
    /^launcher\/crates\/oddity-host\/src\/(environment|runtime_setup)([_.][^/]*)?\.rs$/,
  ],
  "area: login": [/^launcher\/packages\/[^/]+\/backend\/src\/auth\//],
  ci: [/^\.(gitea|github)\/workflows\//, /^scripts\//],
  docs: [/^docs\//, /\.md$/],
};

export function calculateLabels(files) {
  return Object.entries(pathRules)
    .filter(([, patterns]) =>
      files.some((file) => patterns.some((pattern) => pattern.test(file))),
    )
    .map(([label]) => label)
    .sort();
}

export async function syncPrLabels({ request, list }, number) {
  const issueLabels = `/issues/${number}/labels`;
  const [files, current] = await Promise.all([
    list(`/pulls/${number}/files`),
    request("GET", issueLabels),
  ]);
  const paths = files.flatMap(({ filename, previous_filename }) =>
    previous_filename ? [filename, previous_filename] : [filename],
  );
  const desired = new Set(calculateLabels(paths));
  const currentNames = new Set(current.map(({ name }) => name));
  const missing = [...desired].filter((name) => !currentNames.has(name));

  if (missing.length > 0) {
    const available = new Map(
      (await list("/labels")).map(({ name, id }) => [name, id]),
    );
    const labels = missing.map((name) => {
      const id = available.get(name);
      if (!id) throw new Error(`仓库缺少标签: ${name}`);
      return id;
    });
    await request("POST", issueLabels, { labels });
  }
  for (const { id, name } of current) {
    if (Object.hasOwn(pathRules, name) && !desired.has(name)) {
      await request("DELETE", `${issueLabels}/${id}`);
    }
  }
}
