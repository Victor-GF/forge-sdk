#!/usr/bin/env bash
# Gera o release Linux de um jogo da Forja e guarda os símbolos daquele build.
#
# Uso: tools/release.sh <pacote>          (ex.: tools/release.sh racing)
#
# Saída:
#   dist/<pacote>-<versão>-linux/<pacote>            sem símbolos: o que o jogador recebe
#   $FORJA_SYMBOLS/<pacote>/<build-id>/<pacote>      completo, com DWARF: nunca distribuir
#   $FORJA_SYMBOLS/<pacote>/<build-id>/manifesto.txt versão, commit, alvo, data, rustc
#
# FORJA_SYMBOLS é o armazenamento de símbolos (padrão: symbols/ na raiz, fora do git). Guarde-o
# de cada versão publicada: sem o binário completo, os crash reports dela não são simbolizáveis
# (tools/symbolize.sh acha o binário pelo build id do relatório).
#
# Precisa de cargo, readelf e strip (binutils). Roda no Linux (WSL).
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "uso: $0 <pacote>" >&2
    exit 2
fi
package="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"
symbols="${FORJA_SYMBOLS:-$root/symbols}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
target="x86_64-unknown-linux-gnu"
cd "$root"

version="$(cargo pkgid -p "$package" | sed 's/.*[#@]//')"
commit="$(git rev-parse HEAD)"
if [ -n "$(git status --porcelain)" ]; then
    commit="$commit (com alterações locais)"
    echo "aviso: há alterações sem commit; o manifesto registra isso" >&2
fi

# Flags do release. CARGO_ENCODED_RUSTFLAGS substitui as do .cargo/config.toml, por isso o
# --build-id é repetido aqui.
# --remap-path-prefix: sem ele, caminhos absolutos da máquina de build (com o nome do usuário)
# ficam no binário, nas mensagens de panic e no DWARF, mesmo depois do strip. Quando mais de
# uma regra casa, vale a última: a mais específica vem por último.
flags=(
    -C "link-arg=-Wl,--build-id"
    "--remap-path-prefix=$HOME=/home"
    "--remap-path-prefix=$cargo_home=/cargo"
    "--remap-path-prefix=$root=/forja"
)
CARGO_ENCODED_RUSTFLAGS="$(IFS=$'\x1f'; echo "${flags[*]}")"
export CARGO_ENCODED_RUSTFLAGS

cargo build --release --locked --target "$target" -p "$package"
binary="$target_dir/$target/release/$package"

build_id() {
    readelf -n "$1" | awk '/Build ID:/ {print $3}'
}

id="$(build_id "$binary")"
if [ -z "$id" ]; then
    echo "erro: $binary sem build id (o link precisa de --build-id)" >&2
    exit 1
fi

# Símbolos: o binário completo, endereçado pelo build id.
store="$symbols/$package/$id"
mkdir -p "$store"
cp "$binary" "$store/$package"
cat > "$store/manifesto.txt" <<EOF
pacote:  $package
versão:  $version
build:   $id
commit:  $commit
alvo:    $target
data:    $(date -u '+%Y-%m-%d %H:%M:%S') UTC
rustc:   $(rustc --version)
EOF

# Distribuição: cópia sem símbolos nem DWARF. O build id fica (a nota é carregada com o
# programa), e os endereços não mudam: o crash report do jogador casa com o binário guardado.
dist="$root/dist/$package-$version-linux"
rm -rf "$dist"
mkdir -p "$dist"
strip --strip-all -o "$dist/$package" "$binary"

# Conferências: qualquer falha aqui invalida o release.
fail() {
    echo "erro: $1" >&2
    exit 1
}
[ "$(build_id "$dist/$package")" = "$id" ] || fail "o build id mudou no strip"
if readelf -S "$dist/$package" | grep -qE '\.(debug_|symtab)'; then
    fail "dist/ ainda tem símbolos ou DWARF"
fi
for file in "$dist/$package" "$store/$package"; do
    for private in "$HOME" "$root"; do
        if grep -qaF "$private" "$file"; then
            fail "$file contém o caminho '$private' (remap não pegou)"
        fi
    done
done

echo "release:   $dist/$package ($(du -h "$dist/$package" | cut -f1))"
echo "símbolos:  $store/ ($(du -h "$store/$package" | cut -f1))"
echo "build id:  $id"
