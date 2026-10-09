// 演示页冒烟测试：在无头浏览器中打开已构建的页面，确认地图画布与控件渲染、且没有脚本错误。
// 瓦片等网络请求失败不计入（CI / 内网环境可能无法访问底图服务）。
//
// 用法（CI 中执行）：
//   npm run build && npx vite preview --port 4173 &
//   npm i --no-save playwright && npx playwright install chromium
//   node smoke.mjs http://localhost:4173/
//
// 只靠 `vite build` 发现不了运行期问题（例如 MapLibre 6 的 worker 地址未配置），所以单独做这一步。
import { chromium } from 'playwright';

const url = process.argv[2] ?? 'http://localhost:4173/';
const NETWORK = /Failed to load resource|net::|ERR_|AJAXError|Failed to fetch|tile/i;

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || undefined,
  args: ['--use-gl=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
});
const page = await browser.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
page.on('console', (m) => {
  if (m.type() === 'error') errors.push(`console: ${m.text()}`);
});

await page.goto(url, { waitUntil: 'load' });
await page.waitForSelector('canvas.maplibregl-canvas', { timeout: 15000 });
await page.waitForTimeout(5000);

const controls = await page.locator('.maplibregl-ctrl').count();
const fatal = errors.filter((e) => !NETWORK.test(e));
await browser.close();

console.log(`map canvas: ok, controls: ${controls}, ignored network errors: ${errors.length - fatal.length}`);
if (controls === 0 || fatal.length > 0) {
  for (const e of fatal) console.error(e);
  console.error('demo smoke test failed');
  process.exit(1);
}
console.log('demo smoke test passed');
