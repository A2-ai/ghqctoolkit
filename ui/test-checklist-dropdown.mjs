import { chromium } from 'playwright';
import { writeFileSync, mkdirSync } from 'fs';
import { join } from 'path';

const screenshotsDir = '/Users/wescummings/projects/ghqc/ghqctoolkit/ui/test-screenshots';
mkdirSync(screenshotsDir, { recursive: true });

let screenshotIndex = 0;
function screenshotPath(name) {
  screenshotIndex++;
  return join(screenshotsDir, `${String(screenshotIndex).padStart(2, '0')}-${name}.png`);
}

const log = [];
function report(step, message, success = true) {
  const status = success ? 'SUCCESS' : 'FAILURE';
  const entry = `[Step ${step}] ${status}: ${message}`;
  console.log(entry);
  log.push(entry);
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(10000);

  // Step 1: Navigate to http://localhost:3103
  try {
    await page.goto('http://localhost:3103', { waitUntil: 'networkidle' });
    report(1, 'Navigated to http://localhost:3103');
  } catch (e) {
    report(1, `Failed to navigate: ${e.message}`, false);
    await browser.close();
    process.exit(1);
  }

  // Step 2: Take a screenshot
  await page.screenshot({ path: screenshotPath('initial-load'), fullPage: true });
  report(2, 'Screenshot taken after initial load');

  // Step 3: Click the "Create" tab in the left navigation
  try {
    // Try various selectors for the Create tab
    const createTab = await page.locator('text=Create').first();
    await createTab.waitFor({ state: 'visible' });
    await createTab.click();
    report(3, 'Clicked "Create" tab in left navigation');
  } catch (e) {
    report(3, `Failed to click Create tab: ${e.message}`, false);
    // Try alternative selectors
    try {
      await page.click('[data-tab="create"], [href*="create"], button:has-text("Create"), li:has-text("Create")');
      report(3, 'Clicked Create tab via alternative selector');
    } catch (e2) {
      report(3, `Also failed with alternative selector: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(500);

  // Step 4: Take a screenshot
  await page.screenshot({ path: screenshotPath('after-create-tab'), fullPage: true });
  report(4, 'Screenshot taken after clicking Create tab');

  // Step 5: In the milestone section, select an existing milestone from the dropdown
  try {
    // Look for a milestone dropdown/select
    const milestoneSelect = await page.locator('select, [role="combobox"], [role="listbox"]').first();
    await milestoneSelect.waitFor({ state: 'visible', timeout: 5000 });

    // Check what options are available
    const options = await page.locator('option').allTextContents();
    console.log('Milestone options found:', options);

    if (options.length > 0) {
      // Select the first non-placeholder option
      const selectEl = page.locator('select').first();
      const optionValues = await selectEl.locator('option').allTextContents();
      console.log('Select option texts:', optionValues);

      // Pick first option that isn't a placeholder
      const firstReal = optionValues.find(o => o && !o.toLowerCase().includes('select') && !o.toLowerCase().includes('choose') && !o.toLowerCase().includes('--'));
      if (firstReal) {
        await selectEl.selectOption({ label: firstReal });
        report(5, `Selected milestone: "${firstReal}"`);
      } else {
        // Just select the second option (index 1) to skip placeholder
        const allOpts = await selectEl.locator('option').all();
        if (allOpts.length > 1) {
          const val = await allOpts[1].getAttribute('value');
          await selectEl.selectOption({ value: val });
          const selectedText = await allOpts[1].textContent();
          report(5, `Selected milestone at index 1: "${selectedText}"`);
        } else if (allOpts.length > 0) {
          const val = await allOpts[0].getAttribute('value');
          await selectEl.selectOption({ value: val });
          const selectedText = await allOpts[0].textContent();
          report(5, `Selected milestone at index 0: "${selectedText}"`);
        }
      }
    }
  } catch (e) {
    report(5, `Failed to select milestone: ${e.message}`, false);
    // Try clicking a dropdown
    try {
      await page.click('[aria-label*="milestone" i], [placeholder*="milestone" i], [data-testid*="milestone" i]');
      await page.waitForTimeout(300);
      const firstOption = page.locator('[role="option"]').first();
      await firstOption.click();
      report(5, 'Selected milestone via aria role option');
    } catch (e2) {
      report(5, `Also failed alternative milestone selection: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(500);

  // Step 6: Take a screenshot
  await page.screenshot({ path: screenshotPath('after-milestone-select'), fullPage: true });
  report(6, 'Screenshot taken after milestone selection');

  // Step 7: Click the "Create New QC" card (dashed card with + icon)
  try {
    // Try various selectors for the Create New QC card
    const newQCCard = page.locator('text=Create New QC').first();
    await newQCCard.waitFor({ state: 'visible', timeout: 5000 });
    await newQCCard.click();
    report(7, 'Clicked "Create New QC" card');
  } catch (e) {
    report(7, `Failed to find "Create New QC" text: ${e.message}`, false);
    try {
      // Try dashed border card or + icon
      await page.click('[class*="dashed"], button:has-text("+"), [aria-label*="new" i]');
      report(7, 'Clicked new QC card via alternative selector');
    } catch (e2) {
      report(7, `Also failed alternative: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(500);

  // Step 8: Take a screenshot of the modal
  await page.screenshot({ path: screenshotPath('after-new-qc-click'), fullPage: true });
  report(8, 'Screenshot taken of modal (or page after clicking Create New QC)');

  // Step 9: Click the "Select a Checklist" tab in the modal
  try {
    const checklistTab = page.locator('text=Select a Checklist').first();
    await checklistTab.waitFor({ state: 'visible', timeout: 5000 });
    await checklistTab.click();
    report(9, 'Clicked "Select a Checklist" tab');
  } catch (e) {
    report(9, `Failed to find "Select a Checklist" tab: ${e.message}`, false);
    // Try partial text match
    try {
      await page.click('button:has-text("Checklist"), [role="tab"]:has-text("Checklist")');
      report(9, 'Clicked Checklist tab via alternative selector');
    } catch (e2) {
      report(9, `Also failed: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(500);

  // Step 10: Take screenshot - note Name field and checklist content
  await page.screenshot({ path: screenshotPath('select-checklist-tab'), fullPage: true });

  // Read the Name field value and checklist content
  try {
    const nameFieldValue = await page.locator('input[placeholder*="name" i], input[id*="name" i], input[name*="name" i]').first().inputValue();
    report(10, `Screenshot taken. Name field shows: "${nameFieldValue}"`);
  } catch (e) {
    report(10, `Screenshot taken. Could not read Name field: ${e.message}`, false);
  }

  try {
    const checklistContent = await page.locator('textarea').first().inputValue();
    console.log(`Checklist content: "${checklistContent}"`);
    report(10, `Checklist textarea content: "${checklistContent.substring(0, 100)}${checklistContent.length > 100 ? '...' : ''}"`);
  } catch (e) {
    report(10, `Could not read checklist textarea: ${e.message}`, false);
  }

  // Step 11: Clear the Name field and type "My Renamed Checklist"
  try {
    const nameField = page.locator('input[placeholder*="name" i], input[id*="name" i], input[name*="name" i]').first();
    await nameField.waitFor({ state: 'visible', timeout: 5000 });
    await nameField.triple_click().catch(() => nameField.click());
    await nameField.fill('My Renamed Checklist');
    const newValue = await nameField.inputValue();
    report(11, `Cleared Name field and typed "My Renamed Checklist". Field now shows: "${newValue}"`);
  } catch (e) {
    report(11, `Failed to update Name field: ${e.message}`, false);
    // Try finding by label
    try {
      const nameInput = page.getByLabel('Name', { exact: false });
      await nameInput.fill('My Renamed Checklist');
      report(11, 'Filled Name field via label selector');
    } catch (e2) {
      report(11, `Also failed: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(300);

  // Step 12: Take a screenshot
  await page.screenshot({ path: screenshotPath('after-rename'), fullPage: true });
  report(12, 'Screenshot taken after typing "My Renamed Checklist"');

  // Step 13: Click somewhere in the Name field or dropdown arrow to open template dropdown
  try {
    // Look for a dropdown arrow or combo element near the Name field
    const dropdownTrigger = page.locator('[class*="dropdown"] button, [class*="arrow"], select[id*="template" i], [aria-label*="template" i]').first();
    await dropdownTrigger.click();
    report(13, 'Clicked dropdown arrow/trigger to open template dropdown');
  } catch (e) {
    report(13, `Failed to find dropdown trigger: ${e.message}`, false);
    // Try clicking on a select element
    try {
      const selectEl = page.locator('select').first();
      await selectEl.click();
      report(13, 'Clicked select element to show options');
    } catch (e2) {
      report(13, `Also failed: ${e2.message}`, false);
    }
  }
  await page.waitForTimeout(500);

  // Step 14: Take a screenshot showing dropdown options
  await page.screenshot({ path: screenshotPath('dropdown-open'), fullPage: true });

  // List all visible options
  try {
    const options = await page.locator('[role="option"], option, [class*="option" i]').allTextContents();
    report(14, `Screenshot taken. Dropdown options visible: ${JSON.stringify(options)}`);
  } catch (e) {
    report(14, `Screenshot taken. Could not enumerate dropdown options: ${e.message}`);
  }

  // Step 15: Click on one of the template options
  let clickedTemplate = '';
  try {
    // Look for options like "Custom", "Code Review", etc.
    const optionLocators = [
      page.locator('[role="option"]:has-text("Custom")').first(),
      page.locator('[role="option"]:has-text("Code Review")').first(),
      page.locator('[role="option"]').first(),
      page.locator('option:not([value=""]):not([disabled])').first(),
    ];

    let clicked = false;
    for (const locator of optionLocators) {
      try {
        const isVisible = await locator.isVisible();
        if (isVisible) {
          clickedTemplate = await locator.textContent();
          await locator.click();
          clicked = true;
          report(15, `Clicked template option: "${clickedTemplate}"`);
          break;
        }
      } catch (_) {}
    }

    if (!clicked) {
      // Try clicking a select and choosing second option
      const selectEl = page.locator('select').first();
      const opts = await selectEl.locator('option').all();
      if (opts.length > 1) {
        clickedTemplate = await opts[1].textContent();
        const val = await opts[1].getAttribute('value');
        await selectEl.selectOption({ value: val });
        clicked = true;
        report(15, `Selected option via select element: "${clickedTemplate}"`);
      }
    }

    if (!clicked) {
      report(15, 'Could not find any template option to click', false);
    }
  } catch (e) {
    report(15, `Failed to click template option: ${e.message}`, false);
  }
  await page.waitForTimeout(500);

  // Step 16: Take a screenshot immediately after clicking
  await page.screenshot({ path: screenshotPath('after-template-click'), fullPage: true });
  report(16, 'Screenshot taken immediately after clicking template option');

  // Step 17: Report Name field and checklist content
  try {
    const nameFieldValue = await page.locator('input[placeholder*="name" i], input[id*="name" i], input[name*="name" i]').first().inputValue();
    const checklistContent = await page.locator('textarea').first().inputValue();

    report(17, `Name field shows: "${nameFieldValue}"`);
    report(17, `Checklist textarea shows: "${checklistContent.substring(0, 200)}${checklistContent.length > 200 ? '...' : ''}"`);

    if (nameFieldValue !== 'My Renamed Checklist') {
      report(17, `Name field CHANGED from "My Renamed Checklist" to "${nameFieldValue}" after template selection`);
    } else {
      report(17, `Name field RETAINED "My Renamed Checklist" value (did NOT change after template selection)`);
    }
  } catch (e) {
    report(17, `Failed to read final field values: ${e.message}`, false);
  }

  await browser.close();

  console.log('\n=== FINAL REPORT ===');
  log.forEach(l => console.log(l));
  console.log(`\nScreenshots saved to: ${screenshotsDir}`);
})();
