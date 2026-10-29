#!/usr/bin/env bash
# Simboliza endereços `módulo+0x...` de um crash report ou de um log da Forja.
# Funciona com executáveis Windows (PE, alvo windows-gnu: DWARF no .exe) e Linux (ELF).
#
# Uso: tools/symbolize.sh crash/crash_AAAA-MM-DD_HHMMSS.txt target/debug/racing.exe
#      tools/symbolize.sh log.txt target/debug/racing     (avisos "heap geral: ... pilha: ...")
#
# Precisa de objdump e addr2line (binutils). No Windows, se não estiverem no PATH:
#   BINUTILS=/d/develop/ferramentas/msys2/ucrt64/bin tools/symbolize.sh ...
#
# O executável precisa ser exatamente o que gerou o arquivo (mesmo build).
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "uso: $0 <relatorio-ou-log.txt> <executavel.exe>" >&2
    exit 2
fi
input="$1"
exe="$2"
bin="${BINUTILS:+$BINUTILS/}"
module="$(basename "$exe")"
# PE: o deslocamento é relativo à ImageBase. ELF (PIE, padrão do Rust no Linux): relativo a 0.
if "${bin}objdump" -f "$exe" | grep -q "file format elf"; then
    base="0"
else
    base="$("${bin}objdump" -p "$exe" | awk '/^ImageBase/ {print $2}')"
fi

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
