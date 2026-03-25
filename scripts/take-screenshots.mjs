// Playwright script to capture real BearWisdom UI screenshots.
// Usage: npx playwright test --config=playwright.config.ts scripts/take-screenshots.mjs
// Or:    node scripts/take-screenshots.mjs  (with @playwright/test installed)

import { chromium } from 'playwright';

const BASE = 'http://localhost:3030';
const PROJECT = 'F:\\Work\\Projects\\eShop';
const OUT = 'docs/screenshots';

async function main() {
  const browser = await chromium.launch({ headless: true });
  const ctx = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,  // Retina screenshots
  });
  const page = await ctx.newPage();

  // 1. Landing page
  console.log('1. Landing page...');
  await page.goto(BASE);
  await page.waitForTimeout(1000);
  await page.screenshot({ path: `${OUT}/landing.png` });

  // 2. Index eShop — type the path and submit
  console.log('2. Indexing project...');
  const input = page.locator('input[type="text"]').first();
  await input.fill(PROJECT);

  // Find and click the index/open button
  const indexBtn = page.locator('button').filter({ hasText: /index|open|go/i }).first();
  await indexBtn.click();

  // Wait for the explorer to load (graph takes a moment)
  await page.waitForTimeout(8000);
  await page.screenshot({ path: `${OUT}/explorer-full.png` });

  // 3. Graph with concepts sidebar visible
  console.log('3. Graph view...');
  await page.waitForTimeout(3000);
  await page.screenshot({ path: `${OUT}/graph-overview.png` });

  // 4. Click on a concept to filter
  console.log('4. Concept filter...');
  const conceptItems = page.locator('[role="listbox"] button');
  const count = await conceptItems.count();
  if (count > 2) {
    // Click the second concept (skip "All")
    await conceptItems.nth(1).click();
    await page.waitForTimeout(3000);
    await page.screenshot({ path: `${OUT}/concept-filtered.png` });

    // Go back to all
    await conceptItems.first().click();
    await page.waitForTimeout(2000);
  }

  // 5. Click on a node to show detail panel
  console.log('5. Symbol detail...');
  // Click somewhere in the SVG area to select a node
  const svg = page.locator('svg[role="img"]').first();
  const box = await svg.boundingBox();
  if (box) {
    // Click near center where nodes cluster
    await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.45);
    await page.waitForTimeout(2000);
    await page.screenshot({ path: `${OUT}/symbol-detail.png` });
  }

  // 6. Search — symbols mode
  console.log('6. Symbol search...');
  const searchInput = page.locator('input[aria-label]').first();
  await searchInput.click();
  await searchInput.fill('CatalogItem');
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/search-symbols.png` });

  // 7. Switch to AI Search mode (if available)
  console.log('7. AI Search tab...');
  const aiTab = page.locator('button').filter({ hasText: /AI Search/i }).first();
  const aiTabExists = await aiTab.count();
  if (aiTabExists > 0) {
    await aiTab.click();
    await searchInput.fill('how does ordering work');
    await page.waitForTimeout(2000);
    await page.screenshot({ path: `${OUT}/search-ai.png` });
  }

  // 8. Clear search, show clean graph
  console.log('8. Clean graph...');
  await searchInput.fill('');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(1000);
  await page.screenshot({ path: `${OUT}/graph-clean.png` });

  console.log('Done! Screenshots saved to', OUT);
  await browser.close();
}

main().catch(e => {
  console.error(e);
  process.exit(1);
});
