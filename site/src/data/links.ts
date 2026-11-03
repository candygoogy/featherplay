// Every link the site makes to the project's home, in one place.
//
// TODO(before publishing): the GitHub repository isn't public yet and has no
// releases, so `repo` and everything under it (the releases page, issues, the licence files)
// lead nowhere until it is. If the repo's name or owner changes, change `repo` here; every
// page, the download card and the footer read from this file.

export const repo = "https://github.com/candygoogy/featherplay";

export const links = {
  repo,
  /** Every release with its files, checksums and notes (the changelog). */
  releases: `${repo}/releases`,
  issues: `${repo}/issues`,
  licence: `${repo}/blob/main/LICENSE`,
  thirdParty: `${repo}/blob/main/THIRD_PARTY.md`,
  comingFromPear: `${repo}#coming-from-pear`,
  /** The maker. */
  author: "https://github.com/candygoogy",
} as const;
