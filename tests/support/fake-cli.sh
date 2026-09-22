#!/bin/sh
# Test-only Herdr/OpenCode protocol fixture. All writes stay in SIDEBAR_FIXTURE.
set -eu
cd "$SIDEBAR_FIXTURE"
if [ -f hang ]; then
	sleep 30 &
	echo "$!" >sleeper.pid
	wait
fi
case "${0##*/}" in
opencode)
	printf '%s\n' "$@" >>opencode-args
	[ "$1" = api ] || exit 3
	shift
	if [ "$1" = --server ]; then shift 2; fi
	case "$1" in
	get)
		case "$2" in
		*/message\?limit=1)
			if [ -f empty-session ]; then printf '{"data":[]}'; else printf '{"data":[{}]}'; fi
			;;
		*/ses_parent) cat parent.json ;;
		*/ses_fork) cat fork.json ;;
		*) exit 4 ;;
		esac
		;;
	post)
		echo fork >>forks
		if [ -f fail-fork ]; then
			echo 'service unavailable' >&2
			exit 1
		fi
		if [ -f lose-fork-response ]; then
			printf '{'
			exit 0
		fi
		cat fork.json
		;;
	*) exit 5 ;;
	esac
	;;
herdr)
	case "$1 $2" in
	'pane list')
		if [ -f malformed ]; then
			printf 'not-json'
			exit 0
		fi
		printf '{"result":{"panes":['
		cat source.json
		if [ -f side ]; then
			printf ','
			if [ -f not-ready ]; then cat starting.json; else cat side.json; fi
		fi
		printf ']}}\n'
		;;
	'pane process-info') cat process.json ;;
	'plugin pane')
		case "$3" in
		open)
			printf '%s\n' "$@" >>launch-args
			echo open >>opens
			if [ -f reject-open ]; then
				echo 'pane creation rejected' >&2
				exit 1
			fi
			touch side
			if [ -f lose-open-response ]; then
				printf '{'
				exit 0
			fi
			cat opened.json
			;;
		focus)
			printf '%s\n' "$4" >>focuses
			printf '{"result":{}}\n'
			;;
		*) exit 6 ;;
		esac
		;;
	*) exit 7 ;;
	esac
	;;
*) exit 8 ;;
esac
