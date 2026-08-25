# Peter
Prompt written in rust, configured in lua.

## Why Peter?
Inspired by [bob](https://github.com/MordechaiHadad/bob). But this is a prompt. Therefore Peter.

## Installation
Either install from crates.io as `peter_prompt` or directly from this
repository with
```bash
cargo install --git https://github.com/max397574/peter
```

## Usage
Currently just fish shell is supported. Support for more shells is planned
though. Some shorthands for setup are provided. If you want to know what
exactly they do you can check out the `print_init_snippet` function in
`./src/main.rs`.

### Fish Shell
Put the following line into your fish configuration:
```fish
peter_prompt init fish | source
```

## Configuration
There is a default configuration under `default_config.lua`. See there for the
format of new components and modifying components.

You put custom configuration under `~/.config/peter/init.lua`. There are
luaCATS annotations under `annotations.lua` for the components and provided lua
api. This should be up to date after a new release. You also can generate the
annotations using `peter_prompt generate-annotations`.
