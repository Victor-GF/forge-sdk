#!/usr/bin/env bash
# Simboliza a pilha de um crash report da Forja (alvo windows-gnu: símbolos DWARF no .exe).
#
# Uso: tools/symbolize.sh crash/crash_AAAA-MM-DD_HHMMSS.txt target/debug/racing.exe
#
# Precisa de objdump e addr2line (binutils do MSYS2). Se não estiverem no PATH, aponte a pasta:
#   BINUTILS=/d/develop/ferramentas/msys2/ucrt64/bin tools/symbolize.sh ...
#
# O .exe precisa ser exatamente o que gerou o relatório (mesmo build).
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "uso: $0 <relatorio.txt> <executavel.exe>" >&2
    exit 2
fi
report="$1"
exe="$2"
bin="${BINUTILS:+$BINUTILS/}"
module="$(basename "$exe")"
base="$("${bin}objdump" -p "$exe" | awk '/^ImageBase/ {print $2}')"

grep -E "${module}\+0x[0-9a-f]+" "$report" | while read -r line; do
    rva="$(sed -E "s/.*${module}\+(0x[0-9a-f]+).*/\1/" <<< "$line")"
    # -1: endereços de retorno apontam para a instrução depois da chamada.
    address="$(printf '0x%x' $((0x$base + rva - 1)))"
    printf '%-48s %s\n' "$line" "$("${bin}addr2line" -e "$exe" -f -C -p "$address")"
done
