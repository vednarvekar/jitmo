// Builds training/finetune_router_selfcontained.ipynb: the same notebook, but the training data is
// inside it (gzip + base64) so nothing has to be uploaded or mounted from Drive, and the finished
// GGUF is downloaded through the browser. Run from the repo root: node training/make_selfcontained.js
const fs = require('fs');
const zlib = require('zlib');
const nb = JSON.parse(fs.readFileSync('training/finetune_router.ipynb', 'utf8'));
const b64 = (f) => zlib.gzipSync(fs.readFileSync(f), { level: 9 }).toString('base64');
const a = b64('training/router_train.jsonl');
const b = b64('training/router_train_args.jsonl');
const lines = (s) => s.split('\n').map((l, i, all) => l + (i < all.length - 1 ? '\n' : ''));

const dataCell = {
  cell_type: 'code', metadata: {}, execution_count: null, outputs: [],
  source: lines(
    '#@title 2b. The training data (embedded, no upload needed)\n' +
    'import base64, gzip, os\n' +
    'os.makedirs("/content/data", exist_ok=True)\n' +
    `A = "${a}"\n` +
    `B = "${b}"\n` +
    'open("/content/data/router_train.jsonl", "wb").write(gzip.decompress(base64.b64decode(A)))\n' +
    'open("/content/data/router_train_args.jsonl", "wb").write(gzip.decompress(base64.b64decode(B)))\n' +
    'print(sum(1 for _ in open("/content/data/router_train.jsonl")), "pick examples;",\n' +
    '      sum(1 for _ in open("/content/data/router_train_args.jsonl")), "argument examples")'
  ),
};

for (const c of nb.cells) {
  if (c.cell_type !== 'code') continue;
  let s = c.source.join('');
  s = s.replace(/DATA = \[.*\]\n/, 'DATA = ["/content/data/router_train.jsonl", "/content/data/router_train_args.jsonl"]\n');
  s = s.replace('OUT_DIR = "/content/drive/MyDrive/jitmo/out"', 'OUT_DIR = "/content/out"');
  // No Drive: install only, and make the output folder.
  s = s.replace("from google.colab import drive\ndrive.mount('/content/drive')\n", '');
  s = s.replace('!cp /content/{NAME}-Q4_K_M.gguf {OUT_DIR}/\n!ls -l {OUT_DIR}',
    '!cp /content/{NAME}-Q4_K_M.gguf {OUT_DIR}/\n!ls -l {OUT_DIR}\nfrom google.colab import files\nfiles.download(f"{OUT_DIR}/{NAME}-Q4_K_M.gguf")');
  c.source = lines(s);
}
// Insert the data cell right after the install cell (index of the cell containing "pip -q install").
const at = nb.cells.findIndex((c) => c.cell_type === 'code' && c.source.join('').includes('pip -q install'));
nb.cells.splice(at + 1, 0, dataCell);
nb.cells[0].source = lines(
  '# jitmo tool-picker fine-tune (self-contained)\n\n' +
  'Same as `finetune_router.ipynb`, but the training data is embedded below (no Drive, no upload) and the finished model is downloaded by the browser. Runtime > Change runtime type > GPU, then Run all.\n\n' +
  '**Never train on `testset/heldout.txt`.** It is not used here.'
);
fs.writeFileSync('training/finetune_router_selfcontained.ipynb', JSON.stringify(nb, null, 1));
console.log('written', (fs.statSync('training/finetune_router_selfcontained.ipynb').size / 1024).toFixed(0), 'KB');
