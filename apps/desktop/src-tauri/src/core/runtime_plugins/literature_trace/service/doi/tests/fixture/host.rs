//! Protocol substitute only. Core stages, transfers, retires and reaps this child.
pub(super) const SCRIPT: &str = r#"#!/bin/sh
IFS= read -r line || exit 40
printf 'initialize\n' >> '__OBS__/calls'
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
count=0
while IFS= read -r line; do
identity=${line%\"body\":*}
case "$line" in
*'"kind":"shutdown"'*)
printf 'shutdown\n' >> '__OBS__/calls'
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
exit 0 ;;
*'"operation":"doiPreview"'*) ;;
*) exit 41 ;;
esac
count=$((count + 1))
printf 'candidate\n' >> '__OBS__/calls'
printf '%s' "$line" > "__OBS__/request-${count}.json"
handle=${line#*\"relativeHandle\":\"}; handle=${handle%%\"*}
/bin/cp "$handle" "__OBS__/input-${count}.json" || exit 42
printf e > '__OBS__/entered'
IFS= read -r release < '__OBS__/release' || exit 43
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
/bin/cp "__OBS__/reply-${count}.json" "$result" || exit 44
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
if [ '__STOP__' = yes ]; then
# A wrong reply keeps only this fixture's owned root alive for retirement.
kill -STOP "$$"
exit 45
fi
done
exit 46
"#;
