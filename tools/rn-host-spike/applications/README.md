# The development registry

The applications a development session has (docs/adr/0013-launching-applications.mdx): Desktop
Entries in `oxyos/applications/`, found by putting this folder in `XDG_DATA_DIRS`. Each names its
module (`X-OxyOS-Bloom-Module`); icons are the shell preview's artwork.

Each application is a package of its own (docs/adr/0030-installing-applications.mdx), made by
`sample/packages.mjs`, compiled for a runtime. `regress.py` lists these entries for scenarios with a
shell with each packaged application's bundle and runtime its package's (`X-OxyOS-Bloom-Bundle`,
`X-OxyOS-Runtime`), so the dock starts what an installed application would be. An entry naming a
bundle that is not there is not installed: the registry says so and does not list it.

Only applications that run, or whose package is where it will be, are listed: no entry stands for an
application OxyOS cannot start.

- `so.oxy.stash`: Stash, the file manager (OxyHQ/Stash; ADR 0030, decision 13), a system
  application, first in the dock. A package of its own built from its checkout (`STASH_DIR`,
  `sample/packages.mjs`); without it, its entry names where the image will carry its package
  (`/usr/lib/oxyos/apps/so.oxy.stash/`) and the registry says it is not installed.
- `so.oxy.atlas`: Atlas, the App Store (OxyHQ/Atlas; ADR 0030, decisions 13 and 14), a system
  application, in the dock after Stash, with the policy's `store` and `updates` grants. Built from
  its checkout (`ATLAS_DIR`) the same way, and not installed without it.
