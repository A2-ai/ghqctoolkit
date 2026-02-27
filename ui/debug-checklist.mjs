import { chromium } from 'playwright';

(async () => {
  const browser = await chromium.launch({ headless: false, slowMo: 50 });
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
  const page = await context.newPage();

  const logs = [];
  const errors = [];

  page.on('console', msg => logs.push({ type: msg.type(), text: msg.text() }));
  page.on('pageerror', err => errors.push({ message: err.message, stack: err.stack }));

  // ── STEP 1: Navigate ──────────────────────────────────────────────────────
  console.log('\n=== STEP 1: Navigate to http://localhost:3103 ===');
  await page.goto('http://localhost:3103', { waitUntil: 'networkidle' });
  await page.screenshot({ path: '/tmp/screenshot_01_initial.png' });
  console.log('URL:', page.url());
  console.log('Title:', await page.title());

  // ── STEP 2: Click "Create" tab ────────────────────────────────────────────
  console.log('\n=== STEP 2: Click the "Create" tab ===');
  await page.locator('button', { hasText: 'Create' }).first().click();
  await page.waitForTimeout(600);
  await page.screenshot({ path: '/tmp/screenshot_02_create_tab.png' });
  console.log('Screenshot: /tmp/screenshot_02_create_tab.png');

  // ── STEP 3: Select a milestone ────────────────────────────────────────────
  console.log('\n=== STEP 3: Select a milestone ===');
  const milestoneInput = page.locator('input[placeholder="Select a milestone"]');
  await milestoneInput.click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: '/tmp/screenshot_03a_milestone_dropdown_open.png' });
  console.log('Screenshot: /tmp/screenshot_03a_milestone_dropdown_open.png');

  // Use ArrowDown + Enter to select first milestone (avoids Mantine portal visibility issues)
  await milestoneInput.press('ArrowDown');
  await page.waitForTimeout(200);
  await milestoneInput.press('Enter');
  await page.waitForTimeout(600);

  const milestoneVal = await milestoneInput.inputValue().catch(() => '');
  console.log('Selected milestone:', milestoneVal);

  await page.screenshot({ path: '/tmp/screenshot_03b_after_milestone.png' });
  console.log('Screenshot: /tmp/screenshot_03b_after_milestone.png');

  // ── STEP 4: Click "Create New QC" ────────────────────────────────────────
  console.log('\n=== STEP 4: Click "Create New QC" ===');
  await page.waitForTimeout(500);
  const createNewQC = page.locator('text=Create New QC');
  const createNewVis = await createNewQC.isVisible().catch(() => false);
  console.log('"Create New QC" visible:', createNewVis);

  if (!createNewVis) {
    const bodyText = await page.locator('body').innerText().catch(() => '');
    console.log('Page text:', bodyText.substring(0, 600));
  } else {
    await createNewQC.click();
    await page.waitForTimeout(600);
  }
  await page.screenshot({ path: '/tmp/screenshot_04_after_create_new_qc.png' });
  console.log('Screenshot: /tmp/screenshot_04_after_create_new_qc.png');

  // ── STEP 5: Verify modal opened ───────────────────────────────────────────
  console.log('\n=== STEP 5: Verify modal ===');
  const modalVis = await page.locator('[role="dialog"]').isVisible().catch(() => false);
  console.log('Modal visible:', modalVis);

  if (!modalVis) {
    console.log('Modal not open, aborting.');
    console.log('Errors:', JSON.stringify(errors, null, 2));
    await browser.close();
    process.exit(0);
  }

  const modalTabs = await page.locator('[role="dialog"] [role="tab"]').allTextContents();
  console.log('Modal tabs:', modalTabs);

  // ── STEP 6: Click "Select a Checklist" tab ────────────────────────────────
  console.log('\n=== STEP 6: Click "Select a Checklist" tab ===');
  const checklistTab = page.locator('[role="dialog"] [role="tab"]').filter({ hasText: 'Select a Checklist' });
  await checklistTab.click();
  await page.waitForTimeout(800); // Wait for fetchChecklists API call
  await page.screenshot({ path: '/tmp/screenshot_05_checklist_tab.png' });
  console.log('Screenshot: /tmp/screenshot_05_checklist_tab.png');

  // ── STEP 7: Inspect the Name combobox ─────────────────────────────────────
  console.log('\n=== STEP 7: Inspect the Name combobox ===');

  // Use the mantine input ID to locate the name field
  const nameInputById = page.locator('input[placeholder="e.g. Code Review"]');
  const nameVis = await nameInputById.isVisible().catch(() => false);
  console.log('Name input (by placeholder) visible:', nameVis);

  const initVal = await nameInputById.inputValue().catch(() => 'error');
  console.log('Initial name field value:', initVal);

  // Also check what templates loaded
  const checklistInfo = await page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"]');
    if (!dialog) return {};
    const nameInput = dialog.querySelector('input[placeholder="e.g. Code Review"]');
    const textarea = dialog.querySelector('textarea');
    return {
      nameValue: nameInput?.value ?? 'not found',
      textareaValue: textarea?.value?.substring(0, 100) ?? 'not found',
    };
  });
  console.log('Checklist info from DOM:', checklistInfo);

  // ── STEP 8: Type "My Custom Checklist" ────────────────────────────────────
  console.log('\n=== STEP 8: Type "My Custom Checklist" into Name field ===');

  if (nameVis) {
    await nameInputById.click({ clickCount: 3 }); // triple click to select all
    await page.waitForTimeout(100);
    await page.keyboard.type('My Custom Checklist');
    await page.waitForTimeout(300);

    const valAfterType = await nameInputById.inputValue().catch(() => '');
    console.log('Name field value after typing "My Custom Checklist":', valAfterType);

    // Check if dropdown opened during typing
    const dropdownAfterType = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
    console.log('Dropdown open while/after typing:', dropdownAfterType);

    if (dropdownAfterType) {
      const opts = await page.locator('[data-combobox-dropdown] [role="option"]').allTextContents();
      console.log('Options visible in dropdown during typing:', opts);
      await page.screenshot({ path: '/tmp/screenshot_06a_dropdown_during_typing.png' });
      console.log('Screenshot: /tmp/screenshot_06a_dropdown_during_typing.png');
    }
  }

  await page.screenshot({ path: '/tmp/screenshot_06b_after_typing.png' });
  console.log('Screenshot: /tmp/screenshot_06b_after_typing.png');

  // ── STEP 9: Analyze the chevron & try to open the dropdown ───────────────
  console.log('\n=== STEP 9: Chevron analysis and dropdown opening attempt ===');

  // Close any open dropdown first
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);

  // Inspect chevron DOM
  const chevronDetails = await page.evaluate(() => {
    const dialog = document.querySelector('[role="dialog"]');
    if (!dialog) return {};
    const chevrons = dialog.querySelectorAll('[data-combobox-chevron]');
    const rightSections = dialog.querySelectorAll('[data-section="right"]');

    return {
      chevronCount: chevrons.length,
      rightSectionCount: rightSections.length,
      chevrons: Array.from(chevrons).map(el => {
        const rect = el.getBoundingClientRect();
        const parentRect = el.parentElement?.getBoundingClientRect();
        return {
          pointerEvents: getComputedStyle(el).pointerEvents,
          display: getComputedStyle(el).display,
          rect: { top: rect.top, left: rect.left, width: rect.width, height: rect.height },
          parentPointerEvents: el.parentElement ? getComputedStyle(el.parentElement).pointerEvents : 'n/a',
          grandparentPointerEvents: el.parentElement?.parentElement
            ? getComputedStyle(el.parentElement.parentElement).pointerEvents
            : 'n/a',
          parentRect: parentRect
            ? { top: parentRect.top, left: parentRect.left, width: parentRect.width, height: parentRect.height }
            : null,
        };
      }),
    };
  });
  console.log('Chevron DOM details:', JSON.stringify(chevronDetails, null, 2));

  // The chevron has pointer-events: none, so clicking it clicks through to the input wrapper.
  // Let's try clicking at the exact chevron position.
  if (chevronDetails.chevrons && chevronDetails.chevrons.length > 0) {
    const ch = chevronDetails.chevrons[0];
    const cx = ch.rect.left + ch.rect.width / 2;
    const cy = ch.rect.top + ch.rect.height / 2;
    console.log(`Clicking chevron at coordinates (${cx.toFixed(0)}, ${cy.toFixed(0)})`);
    await page.mouse.click(cx, cy);
    await page.waitForTimeout(400);

    const dropdownAfterChevronClick = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
    console.log('Dropdown visible after clicking chevron coordinates:', dropdownAfterChevronClick);

    if (dropdownAfterChevronClick) {
      const opts = await page.locator('[data-combobox-dropdown] [role="option"]').allTextContents();
      console.log('Template options available:', opts);
      await page.screenshot({ path: '/tmp/screenshot_07a_dropdown_open.png' });
      console.log('Screenshot: /tmp/screenshot_07a_dropdown_open.png');
    } else {
      // Try clicking the name input directly
      console.log('Dropdown did not open from chevron click. Trying direct input click...');
      await nameInputById.click();
      await page.waitForTimeout(400);

      const dropdownAfterInput = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
      console.log('Dropdown visible after clicking name input:', dropdownAfterInput);

      if (dropdownAfterInput) {
        const opts = await page.locator('[data-combobox-dropdown] [role="option"]').allTextContents();
        console.log('Template options available:', opts);
      } else {
        console.log('Dropdown still did not open - checking DOM state...');
        const dropdownState = await page.evaluate(() => {
          const dd = document.querySelector('[data-combobox-dropdown]');
          if (!dd) return { found: false };
          const style = getComputedStyle(dd);
          return {
            found: true,
            display: style.display,
            visibility: style.visibility,
            opacity: style.opacity,
            transform: style.transform,
            innerHTML: dd.innerHTML.substring(0, 400),
          };
        });
        console.log('Combobox dropdown DOM state:', JSON.stringify(dropdownState, null, 2));
      }
    }
  }

  await page.screenshot({ path: '/tmp/screenshot_07b_after_dropdown_attempt.png' });
  console.log('Screenshot: /tmp/screenshot_07b_after_dropdown_attempt.png');

  // ── STEP 10: Select "Custom" option ──────────────────────────────────────
  console.log('\n=== STEP 10: Select a template from the dropdown ===');
  const dropdownOpen = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
  console.log('Dropdown currently open:', dropdownOpen);

  if (!dropdownOpen) {
    // Force open
    await nameInputById.click();
    await page.waitForTimeout(300);
  }

  const customOpt = page.locator('[data-combobox-dropdown] [role="option"]').filter({ hasText: 'Custom' });
  const customOptVis = await customOpt.isVisible().catch(() => false);
  console.log('"Custom" option visible:', customOptVis);

  let clickedTemplate = '';
  if (customOptVis) {
    clickedTemplate = 'Custom';
    await customOpt.click({ force: true });
    console.log('Clicked "Custom"');
  } else {
    const allDropdownOpts = page.locator('[data-combobox-dropdown] [role="option"]');
    const count = await allDropdownOpts.count();
    console.log(`Options in dropdown: ${count}`);
    for (let i = 0; i < count; i++) {
      const txt = await allDropdownOpts.nth(i).textContent().catch(() => '');
      const vis = await allDropdownOpts.nth(i).isVisible().catch(() => false);
      console.log(`  Option[${i}]: "${txt}", visible: ${vis}`);
    }
    if (count > 0) {
      const firstVis = await allDropdownOpts.first().isVisible().catch(() => false);
      if (firstVis) {
        clickedTemplate = (await allDropdownOpts.first().textContent().catch(() => '')).trim();
        await allDropdownOpts.first().click({ force: true });
        console.log(`Clicked first option: "${clickedTemplate}"`);
      } else {
        // Try force-click first option regardless
        clickedTemplate = (await allDropdownOpts.first().textContent().catch(() => '')).trim();
        await allDropdownOpts.first().click({ force: true });
        console.log(`Force-clicked first option: "${clickedTemplate}"`);
      }
    }
  }

  await page.waitForTimeout(400);
  await page.screenshot({ path: '/tmp/screenshot_08_after_selection.png' });
  console.log('Screenshot: /tmp/screenshot_08_after_selection.png');

  // ── STEP 11: Final state check ────────────────────────────────────────────
  console.log('\n=== STEP 11: Final state check ===');
  const finalVal = await nameInputById.inputValue().catch(() => 'error');
  console.log('Name field value after template selection:', finalVal);

  const dropdownFinallyOpen = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
  console.log('Dropdown still open:', dropdownFinallyOpen);

  if (clickedTemplate) {
    if (finalVal === clickedTemplate) {
      console.log(`OK: Name field correctly shows "${clickedTemplate}"`);
    } else if (finalVal === 'My Custom Checklist') {
      console.log(`BUG CONFIRMED: Name field still shows "My Custom Checklist" instead of "${clickedTemplate}"`);
    } else {
      console.log(`UNEXPECTED: Name field shows "${finalVal}" (expected "${clickedTemplate}")`);
    }
  }

  // ── STEP 12: Specific bug scenario with fresh state ───────────────────────
  console.log('\n=== STEP 12: Specific bug scenario (type custom → open dropdown → select template) ===');

  // Reset to fresh state
  if (finalVal !== 'Custom') {
    // Click checklist tab to re-enter (but modal stays open)
    // The state is maintained in React state, so reset by clicking elsewhere
  }

  // Type a custom name
  await nameInputById.click({ clickCount: 3 });
  await page.waitForTimeout(100);
  await page.keyboard.type('My Custom Checklist');
  await page.waitForTimeout(200);

  const valStep12 = await nameInputById.inputValue().catch(() => '');
  console.log('Step 12 - after typing custom name:', valStep12);

  // Close dropdown (if opened by typing)
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);

  // Now try to open dropdown by clicking input
  await nameInputById.click();
  await page.waitForTimeout(400);

  const dropdownOpen12 = await page.locator('[data-combobox-dropdown]').isVisible().catch(() => false);
  console.log('Step 12 - dropdown opens after clicking input (after custom typing):', dropdownOpen12);

  if (dropdownOpen12) {
    const availableOpts = await page.locator('[data-combobox-dropdown] [role="option"]').allTextContents();
    console.log('Step 12 - available template options:', availableOpts);

    if (availableOpts.length > 0) {
      const targetOpt = page.locator('[data-combobox-dropdown] [role="option"]').first();
      const targetTxt = (await targetOpt.textContent().catch(() => '')).trim();
      console.log(`Step 12 - clicking template: "${targetTxt}"`);

      await targetOpt.click({ force: true });
      await page.waitForTimeout(400);

      const valAfterSelect12 = await nameInputById.inputValue().catch(() => '');
      console.log('Step 12 - name after template select:', valAfterSelect12);

      if (valAfterSelect12 === targetTxt) {
        console.log(`Step 12 RESULT: OK - Name correctly updated to "${targetTxt}"`);
      } else if (valAfterSelect12 === 'My Custom Checklist') {
        console.log(`Step 12 RESULT: BUG - Name still shows "My Custom Checklist" instead of "${targetTxt}"`);
        console.log('The template selection does NOT reset the name field after user has typed a custom name.');
      } else {
        console.log(`Step 12 RESULT: UNEXPECTED - Name shows "${valAfterSelect12}"`);
      }
    }
  } else {
    console.log('Step 12 - dropdown did not open after clicking input when name has custom text');
    console.log('This may indicate the issue: dropdown cannot be opened to select a template once user has typed something');
  }

  await page.screenshot({ path: '/tmp/screenshot_09_step12_result.png' });
  console.log('Screenshot: /tmp/screenshot_09_step12_result.png');

  // ── FINAL Console Summary ─────────────────────────────────────────────────
  console.log('\n=== Console Summary ===');
  console.log('JS Errors:', JSON.stringify(errors, null, 2));
  const nonDebugLogs = logs.filter(l => l.type !== 'debug');
  console.log('Console logs (non-debug):', JSON.stringify(nonDebugLogs, null, 2));

  await page.waitForTimeout(1000);
  await browser.close();
})();
