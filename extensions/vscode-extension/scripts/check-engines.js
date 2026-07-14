// Fails if a runtime dependency requires a newer VS Code than engines.vscode
// declares. vsce only checks @types/vscode, so without this a languageclient
// bump can ship an extension that installs on VSCodium and dies at runtime.
//
// ponytail: direct runtime deps only, assumes "^1.X.Y" — vscode-languageclient
// is the only vscode-gated dep. Walk node_modules if that stops being true.
const pkg = require('../package.json');

const parse = (range) => {
  const m = /(\d+)\.(\d+)/.exec(range);
  if (!m) throw new Error(`cannot parse vscode range: ${range}`);
  return [Number(m[1]), Number(m[2])];
};

const floor = parse(pkg.engines.vscode);

let bad = false;
for (const dep of Object.keys(pkg.dependencies ?? {})) {
  const req = require(`../node_modules/${dep}/package.json`).engines?.vscode;
  if (!req) continue;

  const need = parse(req);
  if (need[0] > floor[0] || (need[0] === floor[0] && need[1] > floor[1])) {
    console.error(
      `${dep} requires vscode ${req}, above engines.vscode ${pkg.engines.vscode}. ` +
      `Raising the floor drops VSCodium users — check https://github.com/VSCodium/vscodium/releases first.`
    );
    bad = true;
  } else {
    console.log(`${dep} requires vscode ${req} — within ${pkg.engines.vscode}`);
  }
}

process.exit(bad ? 1 : 0);
