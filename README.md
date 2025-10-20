> [!NOTE]
> The ghqc cli is still under development to catch its feature set up to the [R package](https://github.com/a2-ai/ghqc). 
> Additionally compatibility between the two has not bee robustly tested.

# Install

```shell
cargo build --features cli --release
```

# Configuration

Users can configure `ghqc` using a configuration repository. Options include:

* **Checklists** - Each file to be QCed must have a checklist assigned to it to aide the QCer. `ghqc` will include a *Custom*,
 template option. Any additional checklists must be provided in a yaml file or a 
 [GitHub Flavored Markdown](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax) 
 text file within the `checklists` directory of the configuration repository unless otherwise specified by the 
 `checklist_directory` option described below.

 * **Logo** - `ghqc` allows users to include a logo within the resulting QC record PDF. This logo should be found at `logo.png`
 within the configuration repository unless otherwise specified by the `logo_path` option described below.

 * **Options** - Options can be tuned within the `options.yaml`. Available options are:
    * `prepended_checklist_note` - Allows organizations and users to include a note at the top of each checklist.
    * `checklist_display_name` - Don't like the default name for the review list of *checklists*? Change it using this option.
    * `logo_path` - Change the default record logo location from `logo.png`.
    * `checklist_directory` - Change the default checklist location from `checklists`.

## Set-Up

```shell
ghqc configuration setup [GIT]
```

To follow conventions set by the R package while iterating to make the process easier, 
the above command behaves in one of two ways:

### Environment Variable

If `GHQC_CONFIG_REPO` is set, the `GIT` argument is not required. In this case, `ghqc` will clone the repository provided to 
`$XDG_DATA_HOME/ghqc/<repository name>`.

### Argument

If the `GIT` option is not required, `ghqc` will clone the repository provided to `$XDG_DATA_HOME/ghqc/config`.

## Status

```shell
ghqc configuration status
```

Displays the status of the configuration repository like below:
```
== Directory Information ==
📁 directory: /Users/wescummings/projects/ghqc/ghqctoolkit/data/ghqc/config
📦 git repository: a2-ai/ghqc.example_config_repo
Repository is up to date!
📋 Checklists available in 'checklists': 4
✅ Logo found at logo.png
        
== Checklists Summary ==
📌 checklist note: 
│  Note: edit checklist items as needed

- Code Review: 10 checklist items
- Custom: 1 checklist items
- General Script: 3 checklist items
- Report: 7 checklist items
```

## Directory

Other commands will look for the configuration repository based on the following priority:

1. `--config-dir` - Uses the directory provided
2. `GHQC_CONFIG_REPO` - Uses `$XDG_DATA_HOME/ghqc/<repository name>`
3. Otherwise uses `$XDG_DATA_HOME/ghqc/config`

## Example Repository

An example repository is set-up for use and reference at https://github.com/a2-ai/ghqc.example_config_repo.

# Issue

Issues are the unit for QC within `ghqc`. Each QC has an associated GitHub Issue to track the QC.

Issues are grouped into Milestones for organization.

## Create

```
ghqc issue create
```

Providing no arguments will take you through an interactive issue creation.

The first step is to either create a new milestone or select an existing one.
```shell
🚀 Welcome to GHQC Interactive Mode!
? Select or create a milestone:  
  📝 Create new milestone: 
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

Then, select a file. Within a milestone, only one issue can exist for a file to prevent conflicting reviews.
```shell
🚀 Welcome to GHQC Interactive Mode!
> Select or create a milestone: 🎯 Milestone 1
? 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts
> scripts/file_1.qmd
  scripts/file_2.qmd
  🚫 scripts/file_3.qmd (already has issue)
```

After selecting a milestone and a file to be QCed, select a checklist:
```shell
🚀 Welcome to GHQC Interactive Mode!
> Select or create a milestone: 🎯 Milestone 1
> 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts/file_1.qmd
? Select a checklist:  
> 📋 Code Review
  📋 Custom
  📋 General Script
  📋 Report
```

Users can then assign reviewer(s) to the QC:
```shell
🚀 Welcome to GHQC Interactive Mode!
> Select or create a milestone: 🎯 Milestone 1
> 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts/file_1.qmd
> Select a checklist: 📋 Code Review
? 👥 Enter assignee username (use Tab for autocomplete, Enter for none): QCer
  QCer
  Reviewer
```

Lastly, users can add relevant files to the issue:
```shell
🚀 Welcome to GHQC Interactive Mode!
> Select or create a milestone: 🎯 Milestone 1
> 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts/file_1.qmd
> Select a checklist: 📋 Code Review
> 👥 Enter assignee username (use Tab for autocomplete, Enter for none): QCer
> 👥 Enter another assignee (current: QCer, use Tab for autocomplete, Enter to finish): 
? 📁 Enter relevant file path (Tab for autocomplete, directories shown with /, Enter for none):  scripts/
  scripts/file_2.qmd
  scripts/file_3.qmd
```

After preparing the QC, `ghqc` will create the Issue within the GitHub repository:
```shell
🚀 Welcome to GHQC Interactive Mode!
> Select or create a milestone: 🎯 Milestone 1
> 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts/file_1.qmd
> Select a checklist: 📋 Code Review
> 👥 Enter assignee username (use Tab for autocomplete, Enter for none): QCer
> 👥 Enter another assignee (current: QCer, use Tab for autocomplete, Enter to finish): 
? 📁 Enter relevant file path (Tab for autocomplete, directories shown with /, Enter for none): 

✨ Creating issue with:
   📊 Milestone: Milestone 1
   📁 File: scripts/file_1.qmd
   📋 Checklist: Code Review
   👥 Assignees: QCer

✅ Issue created successfully!
https://github.com/my_organization/my_analysis/issues/4
```

The QC Issue has now been created and ready for review by your QCer!

## Comment

To review and provide context about how the files change, `ghqc` provides the ability to comment within the GitHub Issue
and include commit differences. 

```
ghqc issue comment
```

Providing no arguments will take you through an interactive issue comment posting.

The first step is to select an existing Milestone in which your issue exists.
```shell
💬 Welcome to GHQC Comment Mode!
? Select a milestone:  
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

Then, select an issue.
```shell
💬 Welcome to GHQC Comment Mode!
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):  
> scripts/file_1.qmd
  scripts/file_2.qmd
  scripts/file_3.qmd
```

We then select two commits to take the file difference between. It will default the most recent file changing commit and the most recent commented on commit. If those are the same, will select the second most recent file changing commit.
```shell
💬 Welcome to GHQC Comment Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select first commit (press Enter for latest file change):
? Pick commit: 
>    📝 00eadb9b - commit 3
    💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

```shell
💬 Welcome to GHQC Comment Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select first commit (press Enter for latest file change):
> Pick commit: 📝 00eadb9b - commit 3

📝 Select second commit for comparison (press Enter for second file change):
? Pick commit: 
     📝 00eadb9b - commit 3
>   💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

Lastly, you are able to tune which context you'd like to add to the comment by entering a note and/or 
including the commit diff.
```shell
💬 Welcome to GHQC Comment Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select first commit (press Enter for latest file change):
> Pick commit: 📝 00eadb9b - commit 3

📝 Select second commit for comparison (press Enter for second file change):
> Pick commit: 💬📝 bf8e8730 - commit 2

? 📝 Enter optional note for this comment (Enter to skip):
? 📊 Include commit diff in comment? (Y/n)   
```

Then, `ghqc` will post the comment to the selecting Issue within GitHub:
```shell
💬 Welcome to GHQC Comment Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select first commit (press Enter for latest file change):
> Pick commit: 📝 00eadb9b - commit 3

📝 Select second commit for comparison (press Enter for second file change):
> Pick commit: 💬📝 bf8e8730 - commit 2

? 📝 Enter optional note for this comment (Enter to skip):
? 📊 Include commit diff in comment? (Y/n) 

✨ Creating comment with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   📁 File: scripts/file_1.qmd
   📝 Current commit: 00eadb9bf2747dffade4415e63e689c1450261bd
   📝 Previous commit: bf8e8730a66f7be13aa0c895bf8dc2acd033751a
   📊 Include diff: Yes

✅ Comment Created!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-123456789
```

## Approve

Once the review has been completed and implemented, the QCer can approve the Issue.

```
ghqc issue approve
```

Providing no arguments will take you through an interactive issue approval.

First, select a Milestone containing the Issue to approve.
```shell
✅ Welcome to GHQC Approve Mode!
? Select a milestone:  
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

Then, select the issue.
```shell
✅ Welcome to GHQC Approve Mode!
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):  
> scripts/file_1.qmd
  scripts/file_2.qmd
  scripts/file_3.qmd
```

Next, select the commit to approve. Defaults to the latest commit.
```shell
✅ Welcome to GHQC Approve Mode!
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select commit to approve (press Enter for latest):
? Pick commit: 
>   💬📝 00eadb9b - commit 3
    💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

Lastly, optionally include a note to provide additional context to the approval
```shell
✅ Welcome to GHQC Approve Mode!
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select commit to approve (press Enter for latest):
> Pick commit: 💬📝 00eadb9b - commit 3
? 📝 Enter optional note for this comment (Enter to skip):
```

`ghqc` will then post a comment indicating approval and close the issue.
```shell
✅ Welcome to GHQC Approve Mode!
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed

📝 Select commit to approve (press Enter for latest):
> Pick commit: 💬📝 00eadb9b - commit 3
? 📝 Enter optional note for this comment (Enter to skip):

✨ Creating approval with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   📁 File: scripts/file_1.qmd
   📝 Commit: 00eadb9bf2747dffade4415e63e689c1450261bd

✅ Approval created and issue closed!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-987654321
```

## Unapprove

If for some reason an approval should be overturned, we must unapprove the Issue.

```shell
ghqc issue unapprove
```

Providing no arguments will take you through an interactive issue unapproval.

First, select the Milestone containing the Issue to unapprove.
```shell
🚫 Welcome to GHQC Unapprove Mode!
? Select a milestone:  
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

Then, select a closed issue to unapprove.
```shell
🚫 Welcome to GHQC Unapprove Mode!
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):  
> scripts/file_1.qmd
  models/1001.mod
```

Lastly, provide a reason to be included with the unapproval.
```shell
🚫 Welcome to GHQC Unapprove Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
? 📝 Enter reason for unapproval:  Found more changes to be made
```

Then, `ghqc` will post the comment and re-open the Issue.
```shell
🚫 Welcome to GHQC Unapprove Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd
? 📝 Enter reason for unapproval:  Found more changes to be made

✨ Creating unapproval with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   🚫 Reason: Found more changes to be made

🚫 Issue unapproved and reopened!
https://github.com/A2-ai/ghqctoolkit/issues/4#issuecomment-192837465
```

## Status
Provides the status of the issue.

```
ghqc issue status
```

First, select a Milestone containing the Issue of interest.
```shell
✅ Welcome to GHQC Approve Mode!
? Select a milestone:  
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

Then, select the issue.
```shell
✅ Welcome to GHQC Approve Mode!
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):  
> scripts/file_1.qmd
  scripts/file_2.qmd
  scripts/file_3.qmd
```

`ghqc` will then print the status of the issue:

```shell
✅ Welcome to GHQC Approve Mode!
> Select a milestone: 🎯 Milestone 1
> 🎫 Enter issue title (use Tab for autocomplete): scripts/file_1.qmd

- File:         scripts/file_1.qmd
- Branch:       excel
- Issue State:  open
- QC Status:    File change in `bb23a12` not commented
- Git Status:   File is up to date!
- Checklist Summary: 0/5 (0.0%)
    - Code Quality: 0/2 (0.0%)
    - Scientific Review: 0/3 (0.0%)
```

