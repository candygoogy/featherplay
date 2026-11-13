// Every link the site makes to the project's home, in one place. If the repo's name or owner
// changes, change `repo` here; every page, the download card and the footer read from this
// file.

export const repo = "https://github.com/candygoogy/featherplay";

export const links = {
  repo,
  /** Every release with its files, checksums and notes (the changelog). */
  releases: `${repo}/releases`,
  issues: `${repo}/issues`,
  licence: `${repo}/blob/main/LICENSE`,
  thirdParty: `${repo}/blob/main/THIRD_PARTY.md`,
  comingFromPear: `${repo}#coming-from-pear`,
  /** The README's Linux section: the steps, what it needs and the known limits. */
  linux: `${repo}#linux-experimental`,
  /** The README's entry on songs that won't play (a DNS that sends YouTube abroad). */
  cantPlay: `${repo}#many-songs-cant-play-but-they-play-in-a-browser-or-in-pear`,
  /** The maker. */
  author: "https://github.com/candygoogy",
} as const;
