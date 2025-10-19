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

