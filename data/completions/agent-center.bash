# bash completion for agent-center

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
