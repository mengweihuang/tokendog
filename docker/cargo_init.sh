#!/bin/bash

export RUSTUP_DIST_SERVER=https://mirrors.aliyun.com/rustup
export RUSTUP_UPDATE_ROOT=https://mirrors.aliyun.com/rustup/rustup

# curl --proto '=https' --tlsv1.2 -sSf https://mirrors.aliyun.com/repo/rust/rustup-init.sh | sh
# 下载脚本
curl --proto '=https' --tlsv1.2 -sSf https://mirrors.aliyun.com/repo/rust/rustup-init.sh -o rustup-init.sh
# 赋予执行权限并安装
chmod +x rustup-init.sh
sh rustup-init.sh -y
# 清理安装包
rm rustup-init.sh

mkdir -vp $HOME/.cargo
touch $HOME/.cargo/config.toml
echo '[source.crates-io]' > $HOME/.cargo/config.toml
echo "replace-with = 'aliyun'"  >> $HOME/.cargo/config.toml
echo '[source.aliyun]'   >> $HOME/.cargo/config.toml
echo 'registry = "sparse+https://mirrors.aliyun.com/crates.io-index/"'  >> $HOME/.cargo/config.toml

echo '. "$HOME/.cargo/env"' >> ~/.bashrc

# 该脚本执行完后，需要再加载下env，或者重新进入bash
. "$HOME/.cargo/env"
