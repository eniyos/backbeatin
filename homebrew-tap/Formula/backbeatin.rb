# Homebrew formula for Backbeatin
# Install with: brew install backbeatin

class Backbeatin < Formula
  desc "Automatically verify that your Restic and Borg backups can actually be restored"
  homepage "https://github.com/eniyos/backbeatin"
  url "https://github.com/eniyos/backbeatin/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123" # Placeholder - updated by GitHub Actions
  license "MIT"

  depends_on "rust" => :build
  depends_on "docker"

  def install
    system "cargo", "install", "--path", ".", "--locked"
  end

  test do
    system "#{bin}/backbeat", "--help"
  end
end
