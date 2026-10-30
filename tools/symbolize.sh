#!/usr/bin/env bash
# Simboliza endereços `módulo+0x...` de um crash report ou de um log da Forja.
# Funciona com executáveis Windows (PE, alvo windows-gnu: DWARF no .exe) e Linux (ELF).
#
# Uso: tools/symbolize.sh crash/crash_AAAA-MM-DD_HHMMSS.txt
#          o binário é achado em $FORJA_SYMBOLS (padrão: symbols/) pelo build id do arquivo
#          (linha "build:"), como guardado pelo tools/release.sh
#      tools/symbolize.sh log.txt target/debug/racing
#          binário dado à mão (ex.: build de dev); o build id é conferido antes
#
# Precisa de objdump, readelf e addr2line (binutils). No Windows, se não estiverem no PATH:
#   BINUTILS=/d/develop/ferramentas/msys2/ucrt64/bin tools/symbolize.sh ...
#
# O resultado aparece na tela e é salvo ao lado da entrada, trocando `.txt` por `.symbol.txt`
# (crash/crash_X.txt -> crash/crash_X.symbol.txt; sem `.txt`, só acrescenta `.symbol.txt`).
#
# O deslocamento só vale contra o binário exato que gerou o arquivo: build id diferente é erro
# (símbolos errados pareceriam certos).
set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "uso: $0 <relatorio-ou-log.txt> [executavel]" >&2
    exit 2
fi
input="$1"
output="${input%.txt}.symbol.txt"
bin="${BINUTILS:+$BINUTILS/}"
root="$(cd "$(dirname "$0")/.." && pwd)"
symbols="${FORJA_SYMBOLS:-$root/symbols}"

# Build id do relatório ("build:   <hex>") ou do log ("crash report ativo; build: <hex>").
report_id="$(grep -m1 -oE 'build: +[^ ]+' "$input" | awk '{print $2}' || true)"
if ! [[ "$report_id" =~ ^[0-9a-f]+$ ]]; then
    report_id=""
fi

if [ $# -eq 2 ]; then
    exe="$2"
else
    if [ -z "$report_id" ]; then
        echo "erro: $input não tem build id; passe o executável: $0 $input <executavel>" >&2
        exit 1
    fi
    exe="$(find "$symbols" -mindepth 3 -maxdepth 3 -path "*/$report_id/*" -type f ! -name manifesto.txt 2>/dev/null | head -n1)"
    if [ -z "$exe" ]; then
        echo "erro: build $report_id não está em $symbols (release sem símbolos guardados?)" >&2
        exit 1
    fi
fi

is_elf=false
if "${bin}objdump" -f "$exe" | grep -q "file format elf"; then
    is_elf=true
fi

# Conferência do build id (ELF; no PE o formato do GUID no objdump ainda não é lido aqui).
if [ -z "$report_id" ]; then
    echo "aviso: $input sem build id; confira à mão que $exe é do mesmo build" >&2
elif $is_elf; then
    exe_id="$("${bin}readelf" -n "$exe" | awk '/Build ID:/ {print $3}')"
    if [ "$exe_id" != "$report_id" ]; then
        echo "erro: build id diferente: arquivo $report_id, executável ${exe_id:-(nenhum)}" >&2
        exit 1
    fi
else
    echo "aviso: build id não conferido em executável PE" >&2
fi

module="$(basename "$exe")"
# PE: o deslocamento é relativo à ImageBase. ELF (PIE, padrão do Rust no Linux): relativo a 0.
if $is_elf; then
    base="0"
else
    base="$("${bin}objdump" -p "$exe" | awk '/^ImageBase/ {print $2}')"
fi

# Cada linha com endereços é repetida, seguida de um endereço simbolizado por linha.
# Grava num temporário e só renomeia no fim: falha no meio não deixa resultado pela metade.
{
    echo "arquivo:  $input"
    echo "símbolos: $exe"
    echo
    grep -E "${module}\+0x[0-9a-f]+" "$input" | while read -r line; do
        echo "$line"
        for token in $(grep -oE "${module}\+0x[0-9a-f]+" <<< "$line"); do
            rva="${token#*+}"
            # -1: endereços de retorno apontam para a instrução depois da chamada.
            address="$(printf '0x%x' $((0x$base + rva - 1)))"
            # -i: com inline, mostra a cadeia até a função de verdade ("(inlined by) ...").
            printf '    %-28s %s\n' "$token" "$("${bin}addr2line" -e "$exe" -f -C -i -p "$address")"
        done
    done
} | tee "$output.tmp"
mv "$output.tmp" "$output"
echo "salvo em $output" >&2
