# frozen_string_literal: true

path, tag, sha256 = ARGV
abort "usage: #{$PROGRAM_NAME} FORMULA TAG SHA256" unless path && tag && sha256

version = tag.delete_prefix("v")
formula = File.read(path)
replacements = {
  /^  url ".*"$/ => %(  url "https://github.com/professionalgriefer/anki-tui/releases/download/#{tag}/anki-tui-aarch64-apple-darwin.tar.gz"),
  /^  sha256 ".*"$/ => %(  sha256 "#{sha256}"),
  /^  version ".*"$/ => %(  version "#{version}")
}

replacements.each do |pattern, replacement|
  abort "could not find #{pattern.inspect} in #{path}" unless formula.sub!(pattern, replacement)
end

File.write(path, formula)
