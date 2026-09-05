# Third-party attribution

The container framing and Event/Checkpoint readers used by this crate come from the MIT-licensed
`vrf-container` crate in [yakisoba0728/vrfkit](https://github.com/yakisoba0728/vrfkit).

ValCoach consumes that crate through a local source checkout and does not remove or replace its
upstream `LICENSE` or `NOTICE.md` files. The production ReplayData backend remains
`michel-giehl/ValorantReplayParser`; vrfkit is used here only for region-independent probing and
validation.
