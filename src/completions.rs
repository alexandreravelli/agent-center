//! Shell completion scripts, printed by `agent-center completions <shell>`. The Debian package
//! installs the copies kept in `data/completions`, which a test keeps identical.
//!
//! App names are completed with `agent-center list --ids`, so apps added to the catalog later
//! are completed too.

/// The completion script for `shell` (`bash`, `zsh` or `fish`).
pub fn script(shell: &str) -> Option<&'static str> {
    match shell {
        "bash" => Some(BASH),
        "zsh" => Some(ZSH),
        "fish" => Some(FISH),
        _ => None,
    }
}

/// The shells [`script`] knows.
pub const SHELLS: [&str; 3] = ["bash", "zsh", "fish"];

const BASH: &str = r#"# bash completion for agent-center

_agent_center() {
    local cur=${COMP_WORDS[COMP_CWORD]} words
    if [ "$COMP_CWORD" -eq 1 ]; then
        words="list info install update remove categories notify completions help --updates --help --version"
    else
        case ${COMP_WORDS[1]} in
            list) words="--installed --updates --json" ;;
            info) words="--json $(agent-center list --ids 2>/dev/null)" ;;
            install | update | upgrade) words="--dry-run $(agent-center list --ids 2>/dev/null)" ;;
            remove | uninstall) words="--purge --dry-run $(agent-center list --ids 2>/dev/null)" ;;
            notify) words="--now" ;;
            completions) words="bash zsh fish" ;;
            *) return ;;
        esac
    fi
    mapfile -t COMPREPLY < <(compgen -W "$words" -- "$cur")
}

complete -F _agent_center agent-center
"#;

const ZSH: &str = r#"#compdef agent-center
# zsh completion for agent-center

_agent_center() {
    local -a commands apps
    commands=(
        'list:list the apps'
        'info:show an app and the scripts Agent Center runs for it'
        'install:install apps and what they need'
        'update:update apps'
        'remove:remove apps'
        'categories:list the categories'
        'notify:check for updates and show a notification'
        'completions:print a shell completion script'
        'help:show the help'
    )
    if (( CURRENT == 2 )); then
        _describe -t commands 'agent-center command' commands
        compadd -- --updates --help --version
        return
    fi
    case $words[2] in
        info | install | update | upgrade | remove | uninstall)
            apps=(${(f)"$(agent-center list --ids 2>/dev/null)"})
            compadd -a apps
            ;;
    esac
    case $words[2] in
        list) compadd -- --installed --updates --json ;;
        info) compadd -- --json ;;
        install | update | upgrade) compadd -- --dry-run ;;
        remove | uninstall) compadd -- --purge --dry-run ;;
        notify) compadd -- --now ;;
        completions) compadd bash zsh fish ;;
    esac
}

_agent_center "$@"
"#;

const FISH: &str = r#"# fish completion for agent-center

set -l commands list info install update upgrade remove uninstall categories notify completions help
complete -c agent-center -f

complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a list -d 'List the apps'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a info -d 'Show an app and its scripts'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a install -d 'Install apps and what they need'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a update -d 'Update apps'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a remove -d 'Remove apps'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a categories -d 'List the categories'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a notify -d 'Check for updates and notify'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a completions -d 'Print a completion script'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -a help -d 'Show the help'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -l updates -d 'Open the window on the Updates page'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -s h -l help -d 'Show the help'
complete -c agent-center -n "not __fish_seen_subcommand_from $commands" -s V -l version -d 'Show the version'

complete -c agent-center -n '__fish_seen_subcommand_from list' -l installed -d 'Only installed apps'
complete -c agent-center -n '__fish_seen_subcommand_from list' -l updates -d 'Installed apps with a newer version'
complete -c agent-center -n '__fish_seen_subcommand_from list info' -l json -d 'Print JSON'
complete -c agent-center -n '__fish_seen_subcommand_from info install update upgrade remove uninstall' -a '(agent-center list --ids 2>/dev/null)'
complete -c agent-center -n '__fish_seen_subcommand_from install update upgrade remove uninstall' -l dry-run -d 'Show the scripts, run nothing'
complete -c agent-center -n '__fish_seen_subcommand_from remove uninstall' -l purge -d 'Also delete settings and data'
complete -c agent-center -n '__fish_seen_subcommand_from notify' -l now -d 'Check at once'
complete -c agent-center -n '__fish_seen_subcommand_from completions' -a 'bash zsh fish'
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_copies_are_up_to_date() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/completions");
        for (shell, file) in [("bash", "agent-center.bash"), ("zsh", "_agent-center"), ("fish", "agent-center.fish")] {
            let packaged = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
            assert!(
                packaged == script(shell).unwrap_or_default(),
                "data/completions/{file} is out of date: run `agent-center completions {shell} > data/completions/{file}`"
            );
        }
    }
}
