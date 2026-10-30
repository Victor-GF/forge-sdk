#!/usr/bin/env bash
# Limpa tudo o que build, execução e release geram no projeto:
#   - artefatos do cargo: $CARGO_TARGET_DIR (no WSL, ~/.cache/forja-target) e target/ da raiz
#   - crash reports (crash/) e traces (*.trace.json) da raiz
#   - releases (dist/) e símbolos locais (symbols/)
#
# Uso: tools/clean.sh [-n] [-y]
#   -n  só mostra o que seria apagado
#   -y  apaga sem perguntar
#
# symbols/ guarda os binários completos dos releases: sem eles, os crash reports dessas versões
# não são simbolizáveis. Por isso o script pergunta antes (a menos de -y) e nunca apaga um
# $FORJA_SYMBOLS fora do projeto (o armazenamento de verdade).
set -euo pipefail

dry_run=false
assume_yes=false
for arg in "$@"; do
    case "$arg" in
        -n) dry_run=true ;;
        -y) assume_yes=true ;;
        *)
            echo "uso: $0 [-n] [-y]" >&2
            exit 2
            ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
targets=()

# Pastas do cargo só se tiverem o CACHEDIR.TAG que o cargo cria: um CARGO_TARGET_DIR mal
# configurado (ex.: $HOME) nunca é apagado.
add_cargo_dir() {
    if [ -f "$1/CACHEDIR.TAG" ]; then
        targets+=("$1")
    fi
}
add_cargo_dir "$root/target"
if [ -n "${CARGO_TARGET_DIR:-}" ] && [ "$CARGO_TARGET_DIR" != "$root/target" ]; then
    add_cargo_dir "$CARGO_TARGET_DIR"
fi

for path in "$root/crash" "$root/dist" "$root/symbols" "$root"/*.trace.json; do
    if [ -e "$path" ]; then
        targets+=("$path")
    fi
done

if [ -n "${FORJA_SYMBOLS:-}" ] && [ "$FORJA_SYMBOLS" != "$root/symbols" ]; then
    echo "mantido: FORJA_SYMBOLS=$FORJA_SYMBOLS (armazenamento fora do projeto)"
fi

if [ ${#targets[@]} -eq 0 ]; then
    echo "nada para limpar"
    exit 0
fi

echo "a apagar:"
for path in "${targets[@]}"; do
    printf '  %-8s %s\n' "$(du -sh "$path" 2>/dev/null | cut -f1)" "$path"
done
if [ -e "$root/symbols" ]; then
    echo "atenção: symbols/ tem os binários completos dos releases; guarde antes os de versões publicadas"
fi

if $dry_run; then
    exit 0
fi
if ! $assume_yes; then
    if [ ! -t 0 ]; then
        echo "sem terminal para confirmar: use -y" >&2
        exit 1
    fi
    read -r -p "apagar? [s/N] " answer
    if [ "$answer" != "s" ] && [ "$answer" != "S" ]; then
        echo "nada apagado"
        exit 0
    fi
fi

rm -rf -- "${targets[@]}"
echo "limpo"
