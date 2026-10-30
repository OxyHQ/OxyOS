# The development registry

The applications a development session has (docs/adr/0013-launching-applications.mdx): Desktop
Entries in `oxyos/applications/`, found by putting this folder in `XDG_DATA_DIRS`, as
`regress.py` does for scenarios with a shell and `probar.sh` does on the ThinkPad. Each runs a
module of the sample Bloom bundle (`X-OxyOS-Bloom-Module`) with the session's Bloom host; icons
are the shell preview's artwork. Only applications that run are listed: no entry stands for an
application OxyOS cannot start.
