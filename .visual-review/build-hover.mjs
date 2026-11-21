import { readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { build } from '../webui/node_modules/vite/dist/node/index.js'
import react from '../webui/node_modules/@vitejs/plugin-react/dist/index.js'

const root = fileURLToPath(new URL('../webui/', import.meta.url))
const entry = `${root}.hover-entry.jsx`
const fixture = await readFile(new URL('hover-fixture.jsx', import.meta.url), 'utf8')
const result = await build({
  root, configFile: false, publicDir: false,
  plugins: [{
    name: 'tree-hover-review',
    resolveId(id) { if (id === entry) return entry },
    load(id) { if (id === entry) return fixture },
  }, react()],
  build: { write: false, lib: { entry, name: 'TreeHoverReview', formats: ['iife'], fileName: 'review' } },
  define: { 'process.env.NODE_ENV': JSON.stringify('production') },
})
const files = (Array.isArray(result) ? result : [result]).flatMap(bundle => bundle.output)
const js = files.find(file => file.type === 'chunk').code.replace(/<\/script/gi, '<\\/script')
const css = files.find(file => file.fileName.endsWith('.css')).source
await writeFile(new URL('tree-hover-subtree.html', import.meta.url), `<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>投机树 · 悬停子树</title>
<style>${css}
html,body,#review { margin:0; width:100%; height:100%; }
#review { display:flex; flex-direction:column; }
#review .speculation-panel { position:relative; inset:auto; width:100%; height:auto; flex:1; min-height:0; border:0; }
#review .panel-header .icon-button { display:none; }
.review-toolbar { display:flex; flex-wrap:wrap; align-items:center; gap:8px 16px; padding:8px 12px; background:var(--surface-raised); border-bottom:1px solid var(--border); color:var(--text-soft); font-size:13px; }
.review-toolbar nav { display:flex; flex-wrap:wrap; gap:4px; }
.review-toolbar button { border:1px solid var(--border); border-radius:6px; padding:6px 10px; background:var(--surface); color:var(--text); font:inherit; cursor:pointer; }
.review-toolbar button[aria-pressed="true"] { background:var(--tree-authority); color:var(--surface); }
</style>
<div id="review"></div>
<script>${js}</script>
</html>`)
