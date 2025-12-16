class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.16.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.2/pytest-language-server-aarch64-apple-darwin"
      sha256 "a23eb0fa355963c6e8e16ddd49dc2c7159243475eeb9e2ba4d46a547b9fc2be1"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.2/pytest-language-server-x86_64-apple-darwin"
      sha256 "d486ea1bf45970c358aceaac9d0be2d7f29c24216c57a32931fecd94a4457b49"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.2/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "9b9980c5a76a92d1ccefc68cc62ec903c33fc641ace9de4d9f12dae690d1cddd"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.2/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "6daaecec3b8ce525acd143892091c92a6bdd2623bc84d0be255c78b68ad33a00"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
