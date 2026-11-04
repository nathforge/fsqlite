#!/usr/bin/env bash
set -euo pipefail

VERSION="$1"
VERSION_NUM="${VERSION#v}"

DARWIN_ARM64_SHA=$(shasum -a 256 dist/fsqlite_darwin_arm64.tar.gz | awk '{print $1}')
DARWIN_AMD64_SHA=$(shasum -a 256 dist/fsqlite_darwin_amd64.tar.gz | awk '{print $1}')
LINUX_ARM64_SHA=$(shasum -a 256 dist/fsqlite_linux_arm64.tar.gz | awk '{print $1}')
LINUX_AMD64_SHA=$(shasum -a 256 dist/fsqlite_linux_amd64.tar.gz | awk '{print $1}')

cat > /tmp/fsqlite.rb <<RUBY
# typed: false
# frozen_string_literal: true

# This file is generated automatically. DO NOT EDIT.
class Fsqlite < Formula
  desc "Mount a SQLite database as a read-only filesystem"
  homepage "https://github.com/nathforge/fsqlite"
  version "${VERSION_NUM}"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/nathforge/fsqlite/releases/download/${VERSION}/fsqlite_darwin_arm64.tar.gz"
      sha256 "${DARWIN_ARM64_SHA}"
    end
    if Hardware::CPU.intel?
      url "https://github.com/nathforge/fsqlite/releases/download/${VERSION}/fsqlite_darwin_amd64.tar.gz"
      sha256 "${DARWIN_AMD64_SHA}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/nathforge/fsqlite/releases/download/${VERSION}/fsqlite_linux_arm64.tar.gz"
      sha256 "${LINUX_ARM64_SHA}"
    end
    if Hardware::CPU.intel?
      url "https://github.com/nathforge/fsqlite/releases/download/${VERSION}/fsqlite_linux_amd64.tar.gz"
      sha256 "${LINUX_AMD64_SHA}"
    end
  end

  def install
    bin.install "fsqlite"
  end

  def caveats
    <<~EOS
      fsqlite needs a FUSE implementation:
        macOS: brew install --cask macfuse
        Linux: install fuse3 from your package manager
    EOS
  end
end
RUBY

git clone "https://x-access-token:${GH_TOKEN}@github.com/nathforge/homebrew-tap.git" /tmp/homebrew-tap
cp /tmp/fsqlite.rb /tmp/homebrew-tap/fsqlite.rb
cd /tmp/homebrew-tap
git config user.email "noreply@github.com"
git config user.name "github-actions[bot]"
git add fsqlite.rb
git diff --cached --quiet || git commit -m "Update fsqlite to ${VERSION}"
git push
