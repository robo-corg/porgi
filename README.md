# Porgi

A ~~tool~~ corgi to organize and find projects, a porgi.

Probably don't use this yet since its very much a WIP.

## Configuring your porgi

Add the following to `~/.config/porgi/porgi.toml`

```toml
# Add your project directories here
project_dirs = ["~/projects"]

# Set the editor or IDE you (o) will use to open the project
#
# Options:
# - "auto" (default): Use the first working opener
# - "code": Use Visual Studio Code
# - "editor": Use the EDITOR environment variable
# - "config": Use custom command (WIP)
opener = "auto"
```

## Other project finding tools

[projects-cli](https://github.com/webdesserts/projects-cli/tree/master)