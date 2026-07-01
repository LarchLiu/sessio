#!/usr/bin/env node

function parseArgs(argv) {
  const args = new Map();
  for (let i = 2; i < argv.length; i += 1) {
    const token = argv[i];
    if (!token?.startsWith("--")) {
      throw new Error(
        "usage: find-previous-release-tag --repo <owner/repo> --tag <tag> [--api-url <url>] [--token <token>]",
      );
    }

    const equalsIndex = token.indexOf("=");
    if (equalsIndex >= 0) {
      const key = token.slice(2, equalsIndex);
      const value = token.slice(equalsIndex + 1);
      if (!key || !value) {
        throw new Error(
          "usage: find-previous-release-tag --repo <owner/repo> --tag <tag> [--api-url <url>] [--token <token>]",
        );
      }
      args.set(key, value);
      continue;
    }

    const value = argv[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(
        "usage: find-previous-release-tag --repo <owner/repo> --tag <tag> [--api-url <url>] [--token <token>]",
      );
    }
    args.set(token.slice(2), value);
    i += 1;
  }
  return args;
}

function isNumericIdentifier(value) {
  return /^[0-9]+$/.test(value);
}

function parseSemverTag(tag) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(tag);
  if (!match) {
    return null;
  }
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4] ? match[4].split(".") : [],
  };
}

function compareIdentifiers(left, right) {
  const leftNumeric = isNumericIdentifier(left);
  const rightNumeric = isNumericIdentifier(right);
  if (leftNumeric && rightNumeric) {
    return Number(left) - Number(right);
  }
  if (leftNumeric) {
    return -1;
  }
  if (rightNumeric) {
    return 1;
  }
  return left.localeCompare(right);
}

function compareSemver(leftTag, rightTag) {
  const left = parseSemverTag(leftTag);
  const right = parseSemverTag(rightTag);
  if (!left || !right) {
    return leftTag.localeCompare(rightTag);
  }

  for (const key of ["major", "minor", "patch"]) {
    const diff = left[key] - right[key];
    if (diff !== 0) {
      return diff;
    }
  }

  const leftPrerelease = left.prerelease;
  const rightPrerelease = right.prerelease;
  if (leftPrerelease.length === 0 && rightPrerelease.length === 0) {
    return 0;
  }
  if (leftPrerelease.length === 0) {
    return 1;
  }
  if (rightPrerelease.length === 0) {
    return -1;
  }

  const length = Math.max(leftPrerelease.length, rightPrerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftIdentifier = leftPrerelease[index];
    const rightIdentifier = rightPrerelease[index];
    if (leftIdentifier === undefined) {
      return -1;
    }
    if (rightIdentifier === undefined) {
      return 1;
    }
    const diff = compareIdentifiers(leftIdentifier, rightIdentifier);
    if (diff !== 0) {
      return diff;
    }
  }

  return 0;
}

function isPrereleaseTag(tag) {
  return tag.includes("-");
}

function releaseChannel(tag) {
  const parsed = parseSemverTag(tag);
  if (!parsed) {
    return isPrereleaseTag(tag) ? "prerelease" : "stable";
  }
  if (parsed.prerelease.length === 0) {
    return "stable";
  }
  const firstIdentifier = parsed.prerelease[0];
  return isNumericIdentifier(firstIdentifier) ? "prerelease" : firstIdentifier;
}

async function fetchReleases({ apiUrl, repo, token }) {
  const releases = [];
  for (let page = 1; page <= 10; page += 1) {
    const response = await fetch(
      `${apiUrl}/repos/${repo}/releases?per_page=100&page=${page}`,
      {
        headers: {
          Accept: "application/vnd.github+json",
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
      },
    );

    if (!response.ok) {
      const body = await response.text();
      throw new Error(
        `failed to list releases (${response.status} ${response.statusText}): ${body}`,
      );
    }

    const pageItems = await response.json();
    releases.push(...pageItems);
    if (pageItems.length < 100) {
      break;
    }
  }
  return releases;
}

async function main() {
  const args = parseArgs(process.argv);
  const repo = args.get("repo") ?? process.env.GITHUB_REPOSITORY;
  const currentTag = args.get("tag") ?? process.env.GITHUB_REF_NAME;
  const apiUrl = args.get("api-url") ?? process.env.GITHUB_API_URL ?? "https://api.github.com";
  const token = args.get("token") ?? process.env.GITHUB_TOKEN ?? "";

  if (!repo) {
    throw new Error("missing GitHub repository");
  }
  if (!currentTag) {
    throw new Error("missing release tag");
  }

  const currentIsPrerelease = isPrereleaseTag(currentTag);
  const currentChannel = releaseChannel(currentTag);
  const releases = await fetchReleases({ apiUrl, repo, token });
  const previousTag = releases
    .filter((release) => !release.draft)
    .filter((release) => release.tag_name && release.tag_name !== currentTag)
    // Use tag naming as the source of truth for stable/beta/rc grouping so the
    // changelog base stays deterministic even if a GitHub release is edited.
    .filter((release) => isPrereleaseTag(release.tag_name) === currentIsPrerelease)
    .filter((release) => releaseChannel(release.tag_name) === currentChannel)
    .map((release) => release.tag_name)
    .filter((tag) => compareSemver(tag, currentTag) < 0)
    .sort(compareSemver)
    .pop();

  if (previousTag) {
    process.stdout.write(`${previousTag}\n`);
  }
}

await main();
