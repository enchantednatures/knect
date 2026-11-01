# SPDX-License-Identifier: Apache-2.0
# Rendered by hack/release/render-homebrew.sh during the release workflow.
class Knect < Formula
  desc "Knative topology explorer, live policy view, and CloudEvent tap"
  homepage "https://github.com/@@REPOSITORY@@"
  version "@@VERSION@@"
  license "Apache-2.0"

  # Linux/amd64 is the default; platform blocks below override it.
  url "https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_linux_amd64.tar.gz"
  sha256 "@@SHA256_LINUX_AMD64@@"

  on_macos do
    on_arm do
      url "https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_darwin_arm64.tar.gz"
      sha256 "@@SHA256_DARWIN_ARM64@@"
    end

    on_intel do
      url "https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_darwin_amd64.tar.gz"
      sha256 "@@SHA256_DARWIN_AMD64@@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_linux_arm64.tar.gz"
      sha256 "@@SHA256_LINUX_ARM64@@"
    end

    on_intel do
      url "https://github.com/@@REPOSITORY@@/releases/download/@@TAG@@/knect_@@TAG@@_linux_amd64.tar.gz"
      sha256 "@@SHA256_LINUX_AMD64@@"
    end
  end

  def install
    bin.install "knect", "knectd"
  end

  test do
    assert_match "knect", shell_output("#{bin}/knect --version")
    assert_match "knectd", shell_output("#{bin}/knectd --version")
  end
end
