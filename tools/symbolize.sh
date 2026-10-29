#!/usr/bin/env bash
# Simboliza endereços `módulo+0x...` de um crash report ou de um log da Forja
# (alvo windows-gnu: símbolos DWARF no .exe).
#
# Uso: tools/symbolize.sh crash/crash_AAAA-MM-DD_HHMMSS.txt target/debug/racing.exe
#      tools/symbolize.sh log.txt target/debug/racing.exe   (avisos "heap geral: ... pilha: ...")
#
# Precisa de objdump e addr2line (binutils do MSYS2). Se não estiverem no PATH, aponte a pasta:
#   BINUTILS=/d/develop/ferramentas/msys2/ucrt64/bin tools/symbolize.sh ...
#
# O .exe precisa ser exatamente o que gerou o arquivo (mesmo build).
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "uso: $0 <relatorio-ou-log.txt> <executavel.exe>" >&2
    exit 2
fi
input="$1"
exe="$2"
bin="${BINUTILS:+$BINUTILS/}"
module="$(basename "$exe")"
base="$("${bin}objdump" -p "$exe" | awk '/^ImageBase/ {print $2}')"

# Cada linha com endereços é repetida, seguida de um endereço simbolizado por linha.
grep -E "${module}\+0x[0-9a-f]+" "$input" | while read -r line; do
    echo "$line"
    for token in $(grep -oE "${module}\+0x[0-9a-f]+" <<< "$line"); do
        rva="${token#*+}"
        # -1: endereços de retorno apontam para a instrução depois da chamada.
        address="$(printf '0x%x' $((0x$base + rva - 1)))"
        printf '    %-28s %s\n' "$token" "$("${bin}addr2line" -e "$exe" -f -C -p "$address")"
    done
done
