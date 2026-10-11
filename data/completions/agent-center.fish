# fish completion for agent-center

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
