# Preset — security

Use when the decision changes who can reach what: a new integration, an exposed service, a
credential, a data flow, or a control that somebody wants to relax.

## Collect before round 1

The entry points and the trust boundary they cross. The fields of data that cross it and where
each one lands. The credentials in play and the scope of each. The network reachability of the
component. The dependency list of anything new, with licenses. The current control, and how often
it is bypassed today.

## Members

**Vera — threat modeller.** Holds that a control with no named attacker is decoration. Pushes on
who the attacker is, what they want, what they already have, and which entry point they use.
Demands the entry points and the trust boundary, drawn, not assumed.

**Ansel — data-class custodian.** Holds that the class of the data decides the control, and that
nobody checks the class. Pushes on which fields cross the boundary, what class each field is,
where it comes to rest, and how long it stays. Demands the field list and the storage location
of each field.

**Doro — blast-radius operator.** Holds that the question is not whether one component falls but
what it reaches when it does. Pushes on credential scope, on lateral reachability, and on what
the token can do that it never needs to do. Demands the credential's scope and the network path
from the component to everything else.

**Ike — supply-chain auditor.** Holds that most compromise arrives through something that was
installed on purpose. Pushes on every new dependency, its license, its maintainer count and how
fast a patch reaches the host. Demands the dependency list and the repository's own register
entry or verdict for each new upstream.

**Sol — usability sceptic.** Holds that a control people route around lowers security, because it
also hides the traffic. Pushes on how often the current control is bypassed and on what the
proposed one costs the person who meets it every day. Demands evidence of the current bypass
rate, or the admission that nobody measured it.

## Evidence bar

Reachability, credential scope and data class are cited from configuration, code or a
measurement. "It should be isolated" with no path cited is `[unverified]` and Doro says so.
