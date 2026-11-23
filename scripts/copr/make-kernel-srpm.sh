#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 4f1sh3r <git@syn-flut.de>

set -Eeuo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
patch_dir=
disable_list=$repo_root/scripts/copr/configs/t2-disable.list
kernel_release=
buildid=
out_dir=$PWD/out
full_config=0

usage() {
	cat <<EOF
Usage: $0 --buildid SUFFIX [options]

Options:
  --kernel-release RELEASE   Fedora kernel series (for example 7.2) or exact
                             NVR.arch (default: latest in Fedora repos)
  --buildid SUFFIX           Unique suffix after .kait2en (required)
  --patch-dir DIRECTORY      Experiment directory containing patches (required)
  --out DIRECTORY            Output directory (default: ./out)
  --full-config              Do not append the T2 config reduction
EOF
	exit 2
}

while (($#)); do
	case $1 in
	--kernel-release)
		(($# >= 2)) || usage
		kernel_release=$2
		shift 2
		;;
	--buildid)
		(($# >= 2)) || usage
		buildid=$2
		shift 2
		;;
	--patch-dir)
		(($# >= 2)) || usage
		patch_dir=$2
		shift 2
		;;
	--out)
		(($# >= 2)) || usage
		out_dir=$2
		shift 2
		;;
	--full-config)
		full_config=1
		shift
		;;
	*) usage ;;
	esac
done

[[ $buildid =~ ^[A-Za-z0-9][A-Za-z0-9._]*$ ]] || {
	printf 'Build ID must contain only letters, numbers, dots, and underscores: %s\n' \
		"$buildid" >&2
	exit 2
}
[[ -n $patch_dir ]] || {
	printf '%s\n' 'A patch directory is required.' >&2
	exit 2
}
[[ -d $patch_dir ]] || {
	printf 'Patch directory does not exist: %s\n' "$patch_dir" >&2
	exit 1
}
if ((!full_config)); then
	[[ -s $disable_list ]] || {
		printf 'T2 disable list does not exist or is empty: %s\n' "$disable_list" >&2
		exit 1
	}
fi

for command in awk curl dnf find grep install mktemp rpm rpmbuild sed sort; do
	command -v "$command" >/dev/null || {
		printf 'Missing command: %s\n' "$command" >&2
		exit 1
	}
done

mkdir -p "$out_dir"
out_dir=$(cd -- "$out_dir" && pwd -P)
patch_dir=$(cd -- "$patch_dir" && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
download_dir=$work/download
topdir=$work/rpmbuild
mkdir -p "$download_dir" \
	"$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" \
	"$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"

koji_base_url=https://kojipkgs.fedoraproject.org/packages/kernel
if [[ -z $kernel_release ]]; then
	printf '%s\n' 'Downloading latest Fedora kernel source package from repositories'
	dnf download --source --destdir "$download_dir" \
		--disablerepo='*' --enablerepo=fedora --enablerepo=updates kernel
elif [[ $kernel_release =~ ^[0-9]+\.[0-9]+$ ]]; then
	fedora_release=$(rpm -E '%fedora')
	[[ $fedora_release =~ ^[0-9]+$ ]] || {
		printf 'Could not determine the Fedora release: %s\n' \
			"$fedora_release" >&2
		exit 1
	}
	series_pattern=${kernel_release//./\\.}
	mapfile -t versions < <(
		curl -fsSL "$koji_base_url/" |
			sed -n 's/.*href="\([0-9][^"]*\)\/".*/\1/p' |
			grep -E "^${series_pattern}\\.[0-9]+$" |
			LC_ALL=C sort -Vr
	)
	((${#versions[@]} > 0)) || {
		printf 'No Fedora kernel versions found for series %s in Koji.\n' \
			"$kernel_release" >&2
		exit 1
	}

	selected_version=
	selected_release=
	for version in "${versions[@]}"; do
		mapfile -t releases < <(
			curl -fsSL "$koji_base_url/$version/" |
				sed -n 's/.*href="\([^"]*\.fc[0-9][0-9]*\)\/".*/\1/p' |
				grep -E "\\.fc${fedora_release}$" |
				LC_ALL=C sort -Vr
		)
		if ((${#releases[@]} > 0)); then
			selected_version=$version
			selected_release=${releases[0]}
			break
		fi
	done
	[[ -n $selected_release ]] || {
		printf 'No Fedora %s kernel build found for series %s in Koji.\n' \
			"$fedora_release" "$kernel_release" >&2
		exit 1
	}
	kernel_release=$selected_version-$selected_release.x86_64
elif [[ ! $kernel_release =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9A-Za-z][0-9A-Za-z._+~]*\.fc[0-9]+\.x86_64$ ]]; then
	printf 'Expected a series such as 7.2 or exact NVR.arch such as %s: %s\n' \
		'7.2.0-200.fc44.x86_64' "$kernel_release" >&2
	exit 2
fi

if [[ -n $kernel_release ]]; then
	package_release=${kernel_release%.x86_64}
	version=${package_release%%-*}
	release=${package_release#*-}
	source_rpm_name=kernel-$version-$release.src.rpm
	source_url=$koji_base_url/$version/$release/src/$source_rpm_name
	printf 'Downloading Fedora kernel source package %s\n' "$kernel_release"
	curl --fail --location --retry 5 --retry-all-errors --retry-delay 5 \
		--output "$download_dir/$source_rpm_name" "$source_url"
fi

mapfile -t srpms < <(find "$download_dir" -maxdepth 1 -type f \
	-name 'kernel-*.src.rpm' -print | sort -V)
[[ ${#srpms[@]} -eq 1 ]] || {
	printf 'Expected exactly one kernel source RPM, found %d\n' "${#srpms[@]}" >&2
	printf '%s\n' "${srpms[@]}" >&2
	exit 1
}
source_rpm=${srpms[0]}
base_version=$(rpm -qp --qf '%{VERSION}' "$source_rpm")
base_release=$(rpm -qp --qf '%{RELEASE}' "$source_rpm")
base_kver=$base_version-$base_release.x86_64
if [[ -n $kernel_release && $base_kver != "$kernel_release" ]]; then
	printf 'Downloaded kernel release %s does not match requested release %s.\n' \
		"$base_kver" "$kernel_release" >&2
	exit 1
fi
printf 'Selected Fedora kernel base: %s\n' "$base_kver"

rpm -i --nodeps --define "_topdir $topdir" "$source_rpm"
spec=$topdir/SPECS/kernel.spec
sources=$topdir/SOURCES
[[ -s $spec ]] || {
	printf 'Kernel spec was not installed from %s\n' "$source_rpm" >&2
	exit 1
}

declare -A spec_anchors=(
	[linux-kernel-test-source]='^[[:space:]]*Patch999999:[[:space:]]*linux-kernel-test\.patch'
	[linux-kernel-test-apply]='ApplyOptionalPatch[[:space:]]+linux-kernel-test\.patch'
	[kernel-local-source]='^[[:space:]]*Source3001:[[:space:]]*kernel-local'
)
for anchor in "${!spec_anchors[@]}"; do
	grep -Eq "${spec_anchors[$anchor]}" "$spec" || {
		printf 'Fedora kernel spec no longer contains required anchor: %s\n' \
			"$anchor" >&2
		exit 1
	}
done
[[ -f $sources/linux-kernel-test.patch && -f $sources/kernel-local ]] || {
	printf 'Fedora kernel SRPM lacks linux-kernel-test.patch or kernel-local\n' >&2
	exit 1
}

patches=()
if [[ -e $patch_dir/series ]]; then
	[[ -f $patch_dir/series && -s $patch_dir/series ]] || {
		printf 'Patch series is not a non-empty regular file: %s/series\n' \
			"$patch_dir" >&2
		exit 1
	}
	declare -A listed_patches=()
	while IFS= read -r patch_name || [[ -n $patch_name ]]; do
		[[ -n $patch_name && $patch_name != \#* ]] || continue
		[[ $patch_name =~ ^[A-Za-z0-9][A-Za-z0-9._-]*\.patch$ ]] || {
			printf 'Invalid patch name in series: %s\n' "$patch_name" >&2
			exit 1
		}
		[[ -z ${listed_patches[$patch_name]+x} ]] || {
			printf 'Duplicate patch in series: %s\n' "$patch_name" >&2
			exit 1
		}
		[[ -f $patch_dir/$patch_name ]] || {
			printf 'Patch listed in series does not exist: %s\n' "$patch_name" >&2
			exit 1
		}
		listed_patches[$patch_name]=1
		patches+=("$patch_dir/$patch_name")
	done <"$patch_dir/series"
	((${#patches[@]} > 0)) || {
		printf 'Patch series contains no patches: %s/series\n' "$patch_dir" >&2
		exit 1
	}
	while IFS= read -r patch_file; do
		patch_name=${patch_file##*/}
		[[ -n ${listed_patches[$patch_name]+x} ]] || {
			printf 'Patch is not listed in series: %s\n' "$patch_name" >&2
			exit 1
		}
	done < <(find "$patch_dir" -maxdepth 1 -type f -name '*.patch' \
		-print | LC_ALL=C sort)
else
	while IFS= read -r patch_file; do
		patch_name=${patch_file##*/}
		[[ $patch_name =~ ^[0-9]{4}-[A-Za-z0-9][A-Za-z0-9._-]*\.patch$ ]] || {
			printf 'Patch requires a four-digit order prefix without series: %s\n' \
				"$patch_name" >&2
			exit 1
		}
		patches+=("$patch_file")
	done < <(find "$patch_dir" -maxdepth 1 -type f -name '*.patch' \
		-print | LC_ALL=C sort)
	((${#patches[@]} > 0)) || {
		printf 'Experiment contains no patches: %s\n' "$patch_dir" >&2
		exit 1
	}
fi

: >"$sources/linux-kernel-test.patch"
for patch_file in "${patches[@]}"; do
	printf 'Adding %s\n' "${patch_file##*/}"
	cat "$patch_file" >>"$sources/linux-kernel-test.patch"
	printf '\n' >>"$sources/linux-kernel-test.patch"
done

if ((!full_config)); then
	{
		printf '\n# KaiT2en T2-only test-kernel reduction\n'
		while IFS= read -r symbol || [[ -n $symbol ]]; do
			[[ -n $symbol && $symbol != \#* ]] || continue
			[[ $symbol =~ ^CONFIG_[A-Z0-9_]+$ ]] || {
				printf 'Invalid config symbol: %s\n' "$symbol" >&2
				exit 1
			}
			printf '# %s is not set\n' "$symbol"
		done <"$disable_list"
	} >>"$sources/kernel-local"
fi

macro_file=$work/kernel-copr-macros
{
	printf '%%global buildid .kait2en.%s\n' "$buildid"
	for feature in debug debuginfo perf libperf tools doc headers cross_headers \
		selftests kabichk kernel_abi_stablelists configchecks; do
		printf '%%global _without_%s 1\n' "$feature"
	done
} >"$macro_file"
sed -i "1r $macro_file" "$spec"

printf 'Building patched kernel source RPM\n'
rpmbuild -bs --define "_topdir $topdir" "$spec"
mapfile -t built_srpms < <(find "$topdir/SRPMS" -maxdepth 1 -type f \
	-name 'kernel-*.src.rpm' -print | sort -V)
[[ ${#built_srpms[@]} -eq 1 ]] || {
	printf 'Expected exactly one built source RPM, found %d\n' \
		"${#built_srpms[@]}" >&2
	exit 1
}
built_srpm=${built_srpms[0]}
install -m 0644 "$built_srpm" "$out_dir/"
output_srpm=$out_dir/${built_srpm##*/}
nvr=$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$output_srpm")
kver=$(rpm -qp --qf '%{VERSION}-%{RELEASE}.x86_64' "$output_srpm")

printf 'srpm=%s\n' "$output_srpm"
printf 'srpm_name=%s\n' "${output_srpm##*/}"
printf 'base_kver=%s\n' "$base_kver"
printf 'nvr=%s\n' "$nvr"
printf 'kver=%s\n' "$kver"
if [[ -n ${GITHUB_OUTPUT:-} ]]; then
	{
		printf 'srpm=%s\n' "$output_srpm"
		printf 'srpm_name=%s\n' "${output_srpm##*/}"
		printf 'base_kver=%s\n' "$base_kver"
		printf 'nvr=%s\n' "$nvr"
		printf 'kver=%s\n' "$kver"
	} >>"$GITHUB_OUTPUT"
fi
