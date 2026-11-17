use corvette_track::{IdSource, Observation, Sighting, Track, TrackConfig, Tracker};
use corvette_yolo::round_f64_to_f32;
use std::collections::{BTreeMap, BTreeSet};
use std::ptr;

type TagId = (u64, Option<u64>);
type ProbePair = (Vec<Sighting<u64>>, Vec<Sighting<u64>>);

const FIXTURE: &str = include_str!("fixtures/tracker_cases.txt");

const HAND_ROWS: &str = r"# steady-object-becomes-real: one car moves 2 px a frame; created initializing, matched in the second pass, real at the third frame with id 1
scenario steady-object-becomes-real max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 1
det 3 car 106 100 146 180
result ok tracks 1
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a365142505ce5 4059000000000000 40621b28a1282e73 4066800000000000
frame 5 dets 1
det 4 car 108 100 148 180
result ok tracks 1
track car 5 4 0 1 1 1 4 1@1/2@2/3@3/4@4 405ac1d50812646a 4059000000000000 406260ea84093236 4066800000000000
frame 6 dets 1
det 5 car 110 100 150 180
result ok tracks 1
track car 6 5 0 1 1 1 5 2@2/3@3/4@4/5@5 405b51549220a275 4059000000000000 4062a8aa4910513b 4066800000000000
end

# no-detections-age-out: a real track with no detections loses one count a frame, stays listed at -1 as inactive and is removed at the next frame
scenario no-detections-age-out max 5 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 1
det 3 car 106 100 146 180
result ok tracks 1
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a365142505ce5 4059000000000000 40621b28a1282e73 4066800000000000
frame 5 dets 1
det 4 car 108 100 148 180
result ok tracks 1
track car 5 4 0 1 1 1 4 1@1/2@2/3@3/4@4 405ac1d50812646a 4059000000000000 406260ea84093236 4066800000000000
frame 6 dets 0
result ok tracks 1
track car 4 5 0 1 1 1 4 1@1/2@2/3@3/4@4 405b1d5bd3989a63 4059000000000000 40628eade9cc4d32 4066800000000000
frame 7 dets 0
result ok tracks 1
track car 3 6 0 1 1 1 4 1@1/2@2/3@3/4@4 405b78e29f1ed05c 4059000000000000 4062bc714f8f682e 4066800000000000
frame 8 dets 0
result ok tracks 1
track car 2 7 0 1 1 1 4 1@1/2@2/3@3/4@4 405bd4696aa50655 4059000000000000 4062ea34b552832a 4066800000000000
frame 9 dets 0
result ok tracks 1
track car 1 8 0 1 1 1 4 1@1/2@2/3@3/4@4 405c2ff0362b3c4e 4059000000000000 406317f81b159e26 4066800000000000
frame 10 dets 0
result ok tracks 1
track car 0 9 0 1 1 1 4 1@1/2@2/3@3/4@4 405c8b7701b17247 4059000000000000 406345bb80d8b922 4066800000000000
frame 11 dets 0
result ok tracks 1
track car -1 10 0 1 1 0 4 1@1/2@2/3@3/4@4 405ce6fdcd37a840 4059000000000000 4063737ee69bd41e 4066800000000000
frame 12 dets 0
result ok tracks 0
frame 13 dets 0
result ok tracks 0
end

# other-label-does-not-match: a person at the car's box does not hit the car; it starts its own track
scenario other-label-does-not-match max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 1
det 3 person 108 100 148 180
result ok tracks 2
track car 2 3 0 1 1 1 2 0@0/1@1/2@2 4059def1826a439f 4059000000000000 4061ef78c13521d0 4066800000000000
track person 1 0 1 2 - 0 3 3@0 405b000000000000 4059000000000000 4062800000000000 4066800000000000
frame 5 dets 1
det 4 person 110 100 150 180
result ok tracks 2
track car 1 4 0 1 1 1 2 0@0/1@1/2@2 405a009a90e7d95b 4059000000000000 4062004d4873ecae 4066800000000000
track person 2 1 1 2 - 0 4 3@0/4@1 405b61fc780e1fc8 4059000000000000 4062b0fe3c070fe4 4066800000000000
end

# nearest-of-two-detections-wins: two detections near one real track: the nearer one hits it whatever the list order, the other starts a track
scenario nearest-of-two-detections-wins max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 2
det 3 car 140 100 180 180
det 4 car 108 100 148 180
result ok tracks 2
track car 4 3 0 1 1 1 4 0@0/1@1/2@2/4@3 405a7bc21091abc3 4059000000000000 40623de10848d5e2 4066800000000000
track car 1 0 1 2 - 0 3 3@0 4061800000000000 4059000000000000 4066800000000000 4066800000000000
frame 5 dets 2
det 5 car 110 100 150 180
det 6 car 142 100 182 180
result ok tracks 2
track car 5 4 0 1 1 1 5 1@1/2@2/4@3/5@4 405b32bea95388f5 4059000000000000 4062995f54a9c47b 4066800000000000
track car 2 1 1 2 - 0 6 3@0/6@1 4061b0fe3c070fe4 4059000000000000 4066b0fe3c070fe4 4066800000000000
end

# nearest-of-two-tracks-wins: one detection between two real tracks hits the nearer; the other only loses a count
scenario nearest-of-two-tracks-wins max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 2
det 0 car 100 100 140 180
det 1 car 150 100 190 180
result ok tracks 2
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
track car 1 0 1 2 - 0 1 1@0 4062c00000000000 4059000000000000 4067c00000000000 4066800000000000
frame 2 dets 2
det 2 car 102 100 142 180
det 3 car 152 100 192 180
result ok tracks 2
track car 2 1 1 1 - 0 2 0@0/2@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
track car 2 1 1 2 - 0 3 1@0/3@1 4062f0fe3c070fe4 4059000000000000 4067f0fe3c070fe4 4066800000000000
frame 3 dets 2
det 4 car 104 100 144 180
det 5 car 154 100 194 180
result ok tracks 2
track car 3 2 0 1 1 1 4 0@0/2@1/4@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
track car 3 2 0 2 2 1 5 1@0/3@1/5@2 40631ea439f656f2 4059000000000000 40681ea439f656f2 4066800000000000
frame 4 dets 1
det 6 car 159 100 199 180
result ok tracks 2
track car 2 3 0 1 1 1 4 0@0/2@1/4@2 4059def1826a439f 4059000000000000 4061ef78c13521d0 4066800000000000
track car 4 3 0 2 2 1 6 1@0/3@1/5@2/6@3 40638f3d3bd92999 4059000000000000 40688f3d3bd92999 4066800000000000
frame 5 dets 2
det 7 car 110 100 150 180
det 8 car 161 100 201 180
result ok tracks 2
track car 3 4 0 1 1 1 7 0@0/2@1/4@2/7@4 405ad0992156efda 4059000000000000 4062684c90ab77ed 4066800000000000
track car 5 4 0 2 2 1 8 3@1/5@2/6@3/8@4 4063f599bcfa0d9d 4059000000000000 4068f599bcfa0d9d 4066800000000000
end

# real-track-wins-over-closer-initializing: a detection nearer to an initializing track than to a real one still goes to the real track in the first pass
scenario real-track-wins-over-closer-initializing max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 2
det 3 car 106 100 146 180
det 4 car 130 100 170 180
result ok tracks 2
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a365142505ce5 4059000000000000 40621b28a1282e73 4066800000000000
track car 1 0 1 2 - 0 4 4@0 4060400000000000 4059000000000000 4065400000000000 4066800000000000
frame 5 dets 1
det 5 car 128 100 168 180
result ok tracks 2
track car 5 4 0 1 1 1 5 1@1/2@2/3@3/5@4 405d77e91f268263 4059000000000000 4063bbf48f934132 4066800000000000
track car 0 1 1 2 - 0 4 4@0 4060400000000000 4059000000000000 4065400000000000 4066800000000000
frame 6 dets 2
det 6 car 108 100 148 180
det 7 car 130 100 170 180
result ok tracks 2
track car 6 5 0 1 1 1 7 2@2/3@3/5@4/7@5 405fae845304fb28 4059000000000000 4064d74229827d94 4066800000000000
track car 1 2 1 2 - 0 6 4@0/6@2 405c4a26d764a26e 4059000000000000 406325136bb25137 4066800000000000
end

# real-and-initializing-matched-in-one-frame: the first pass hits the real track and the second pass hits the initializing one in the same frame; the second becomes real with id 2
scenario real-and-initializing-matched-in-one-frame max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 2
det 3 car 106 100 146 180
det 4 car 500 100 540 180
result ok tracks 2
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a365142505ce5 4059000000000000 40621b28a1282e73 4066800000000000
track car 1 0 1 2 - 0 4 4@0 407f400000000000 4059000000000000 4080e00000000000 4066800000000000
frame 5 dets 2
det 5 car 108 100 148 180
det 6 car 502 100 542 180
result ok tracks 2
track car 5 4 0 1 1 1 5 1@1/2@2/3@3/5@4 405ac1d50812646a 4059000000000000 406260ea84093236 4066800000000000
track car 2 1 1 2 - 0 6 4@0/6@1 407f587f1e0387f2 4059000000000000 4080ec3f8f01c3f9 4066800000000000
frame 6 dets 2
det 7 car 110 100 150 180
det 8 car 504 100 544 180
result ok tracks 2
track car 6 5 0 1 1 1 7 2@2/3@3/5@4/7@5 405b51549220a275 4059000000000000 4062a8aa4910513b 4066800000000000
track car 3 2 0 2 2 1 8 4@0/6@1/8@2 407f6f521cfb2b79 4059000000000000 4080f7a90e7d95bd 4066800000000000
end

# ids-follow-the-pick-order: two initializing tracks become real in one frame; the one picked first (smallest distance) takes id 1 although it is listed second
scenario ids-follow-the-pick-order max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 2
det 0 car 100 100 140 180
det 1 car 600 100 640 180
result ok tracks 2
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
track car 1 0 1 2 - 0 1 1@0 4082c00000000000 4059000000000000 4084000000000000 4066800000000000
frame 2 dets 2
det 2 car 101 100 141 180
det 3 car 601 100 641 180
result ok tracks 2
track car 2 1 1 1 - 0 2 0@0/2@1 405930fe3c070fe4 4059000000000000 4061987f1e0387f2 4066800000000000
track car 2 1 1 2 - 0 3 1@0/3@1 4082c61fc780e1fc 4059000000000000 4084061fc780e1fc 4066800000000000
frame 3 dets 2
det 4 car 130 100 170 180
det 5 car 602 100 642 180
result ok tracks 2
track car 3 2 0 1 2 1 4 0@0/2@1/4@2 405d3d3f891a6159 4059000000000000 40639e9fc48d30ac 4066800000000000
track car 3 2 0 2 1 1 5 1@0/3@1/5@2 4082cbd4873ecade 4059000000000000 40840bd4873ecade 4066800000000000
end

# ties-go-to-the-first-detection: two identical detections: the first in the list hits the track, the second starts a track
scenario ties-go-to-the-first-detection max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 2
det 3 car 107 100 147 180
det 4 car 107 100 147 180
result ok tracks 2
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a5909a9710454 4059000000000000 40622c84d4b8822a 4066800000000000
track car 1 0 1 2 - 0 4 4@0 405ac00000000000 4059000000000000 4062600000000000 4066800000000000
frame 5 dets 2
det 5 car 109 100 149 180
det 6 car 109 100 149 180
result ok tracks 2
track car 5 4 0 1 1 1 5 1@1/2@2/3@3/5@4 405afa49d8b2f6b0 4059000000000000 40627d24ec597b58 4066800000000000
track car 2 1 1 2 - 0 6 4@0/6@1 405b21fc780e1fc8 4059000000000000 406290fe3c070fe4 4066800000000000
end

# three-new-detections-in-order: three detections on an empty tracker become three tracks in list order with initializing ids 1, 2, 3
scenario three-new-detections-in-order max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 3
det 0 car 100 100 140 180
det 1 person 300 100 340 180
det 2 car 500 100 540 180
result ok tracks 3
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
track person 1 0 1 2 - 0 1 1@0 4072c00000000000 4059000000000000 4075400000000000 4066800000000000
track car 1 0 1 3 - 0 2 2@0 407f400000000000 4059000000000000 4080e00000000000 4066800000000000
end

# pruned-track-is-replaced-with-new-ids: after a track is removed the same box starts a new track with the next ids
scenario pruned-track-is-replaced-with-new-ids max 3 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 0
result ok tracks 1
track car 2 3 0 1 1 1 2 0@0/1@1/2@2 4059def1826a439f 4059000000000000 4061ef78c13521d0 4066800000000000
frame 5 dets 0
result ok tracks 1
track car 1 4 0 1 1 1 2 0@0/1@1/2@2 405a009a90e7d95b 4059000000000000 4062004d4873ecae 4066800000000000
frame 6 dets 0
result ok tracks 1
track car 0 5 0 1 1 1 2 0@0/1@1/2@2 405a22439f656f17 4059000000000000 40621121cfb2b78c 4066800000000000
frame 7 dets 0
result ok tracks 1
track car -1 6 0 1 1 0 2 0@0/1@1/2@2 405a43ecade304d3 4059000000000000 406221f656f1826a 4066800000000000
frame 8 dets 0
result ok tracks 0
frame 9 dets 0
result ok tracks 0
frame 10 dets 1
det 3 car 100 100 140 180
result ok tracks 1
track car 1 0 1 2 - 0 3 3@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 11 dets 1
det 4 car 101 100 141 180
result ok tracks 1
track car 2 1 1 2 - 0 4 3@0/4@1 405930fe3c070fe4 4059000000000000 4061987f1e0387f2 4066800000000000
frame 12 dets 1
det 5 car 102 100 142 180
result ok tracks 1
track car 3 2 0 2 2 1 5 3@0/4@1/5@2 40595ea439f656f2 4059000000000000 4061af521cfb2b79 4066800000000000
end

# nan-in-first-pass: with delay 0 a zero-width box is a real track at once; the same box again gives a NaN distance, the update stops after pruning and stepping, and the next frame goes on
scenario nan-in-first-pass max 25 delay 0 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 100 180
result ok tracks 1
track car 1 0 0 1 1 1 0 0@0 4059000000000000 4059000000000000 4059000000000000 4066800000000000
frame 2 dets 1
det 1 car 100 100 100 180
result nan tracks 1
track car 0 1 0 1 1 1 0 0@0 4059000000000000 4059000000000000 4059000000000000 4066800000000000
frame 3 dets 1
det 2 car 300 100 340 180
result ok tracks 2
track car -1 2 0 1 1 0 0 0@0 4059000000000000 4059000000000000 4059000000000000 4066800000000000
track car 1 0 0 2 2 1 2 2@0 4072c00000000000 4059000000000000 4075400000000000 4066800000000000
frame 4 dets 1
det 3 car 302 100 342 180
result ok tracks 1
track car 2 1 0 2 2 1 3 2@0/3@1 4072d87f1e0387f2 4059000000000000 4075587f1e0387f2 4066800000000000
end

# nan-in-second-pass: the first pass hits the real track, then the second pass finds a NaN against the zero-width initializing track: the hit stays, no track is created
scenario nan-in-second-pass max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
frame 2 dets 1
det 1 car 102 100 142 180
result ok tracks 1
track car 2 1 1 1 - 0 1 0@0/1@1 405961fc780e1fc8 4059000000000000 4061b0fe3c070fe4 4066800000000000
frame 3 dets 1
det 2 car 104 100 144 180
result ok tracks 1
track car 3 2 0 1 1 1 2 0@0/1@1/2@2 4059bd4873ecade3 4059000000000000 4061dea439f656f2 4066800000000000
frame 4 dets 2
det 3 car 106 100 146 180
det 4 car 400 100 400 180
result ok tracks 2
track car 4 3 0 1 1 1 3 0@0/1@1/2@2/3@3 405a365142505ce5 4059000000000000 40621b28a1282e73 4066800000000000
track car 1 0 1 2 - 0 4 4@0 4079000000000000 4059000000000000 4079000000000000 4066800000000000
frame 5 dets 2
det 5 car 108 100 148 180
det 6 car 400 100 400 180
result nan tracks 2
track car 5 4 0 1 1 1 5 1@1/2@2/3@3/5@4 405ac1d50812646a 4059000000000000 406260ea84093236 4066800000000000
track car 0 1 1 2 - 0 4 4@0 4079000000000000 4059000000000000 4079000000000000 4066800000000000
frame 6 dets 1
det 7 car 110 100 150 180
result ok tracks 2
track car 6 5 0 1 1 1 7 2@2/3@3/5@4/7@5 405b51549220a275 4059000000000000 4062a8aa4910513b 4066800000000000
track car -1 2 1 2 - 0 4 4@0 4079000000000000 4059000000000000 4079000000000000 4066800000000000
end

# empty-frames-on-an-empty-tracker: empty frames change nothing; the first detection after them is the first track
scenario empty-frames-on-an-empty-tracker max 25 delay 2 past 4 threshold 4004000000000000 r 400b333333333333 q 3fb999999999999a
frame 1 dets 0
result ok tracks 0
frame 2 dets 0
result ok tracks 0
frame 3 dets 0
result ok tracks 0
frame 4 dets 1
det 0 car 100 100 140 180
result ok tracks 1
track car 1 0 1 1 - 0 0 0@0 4059000000000000 4059000000000000 4061800000000000 4066800000000000
end

";

#[derive(Debug)]
struct ExpectedTrack {
    label: String,
    hit_counter: i64,
    age: u64,
    initializing: bool,
    initializing_id: u64,
    id: Option<u64>,
    active: bool,
    last_tag: u64,
    past: Vec<(u64, u64)>,
    estimate: [u64; 4],
}

#[derive(Debug)]
struct Frame {
    number: usize,
    detections: Vec<(u64, String, [i32; 4])>,
    raised: bool,
    expected_tracks: usize,
    tracks: Vec<ExpectedTrack>,
}

#[derive(Debug)]
struct Scenario {
    name: String,
    max: u32,
    delay: u32,
    past: u32,
    threshold: f64,
    r: u64,
    q: u64,
    frames: Vec<Frame>,
}

fn broken(tag: &str, line_number: usize, detail: &str) -> ! {
    panic!("{tag} line {line_number}: {detail}");
}

fn as_u64(text: &str, tag: &str, line_number: usize, what: &str) -> u64 {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_i64(text: &str, tag: &str, line_number: usize, what: &str) -> i64 {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_i32(text: &str, tag: &str, line_number: usize, what: &str) -> i32 {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_u32(text: &str, tag: &str, line_number: usize, what: &str) -> u32 {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_usize(text: &str, tag: &str, line_number: usize, what: &str) -> usize {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_hex64(text: &str, tag: &str, line_number: usize, what: &str) -> u64 {
    u64::from_str_radix(text, 16).unwrap_or_else(|_| broken(tag, line_number, what))
}

fn parse_past(text: &str) -> Vec<(u64, u64)> {
    if text == "-" {
        return Vec::new();
    }
    text.split('/')
        .map(|entry| {
            let (tag, age) = entry.split_once('@').expect("past entry");
            (
                tag.parse().expect("past tag"),
                age.parse().expect("past age"),
            )
        })
        .collect()
}

fn parse_expected_track(fields: &[&str], tag: &str, line_number: usize) -> ExpectedTrack {
    ExpectedTrack {
        label: fields[1].to_string(),
        hit_counter: as_i64(fields[2], tag, line_number, "hit counter"),
        age: as_u64(fields[3], tag, line_number, "age"),
        initializing: fields[4] == "1",
        initializing_id: as_u64(fields[5], tag, line_number, "initializing id"),
        id: if fields[6] == "-" {
            None
        } else {
            Some(as_u64(fields[6], tag, line_number, "id"))
        },
        active: fields[7] == "1",
        last_tag: as_u64(fields[8], tag, line_number, "last tag"),
        past: parse_past(fields[9]),
        estimate: [
            as_hex64(fields[10], tag, line_number, "estimate slot zero"),
            as_hex64(fields[11], tag, line_number, "estimate slot one"),
            as_hex64(fields[12], tag, line_number, "estimate slot two"),
            as_hex64(fields[13], tag, line_number, "estimate slot three"),
        ],
    }
}

fn parse_rows(tag: &str, text: &str) -> Vec<Scenario> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    let mut scenario: Option<Scenario> = None;
    let mut frame: Option<Frame> = None;
    let finish_frame = |scenario: &mut Option<Scenario>, frame: &mut Option<Frame>| {
        if let (Some(scenario), Some(owned)) = (scenario.as_mut(), frame.take()) {
            scenario.frames.push(owned);
        }
    };
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(' ').collect();
        match fields[0] {
            "scenario" => {
                if fields.len() != 14 {
                    broken(tag, line_number, "scenario fields");
                }
                scenario = Some(Scenario {
                    name: fields[1].to_string(),
                    max: as_u32(fields[3], tag, line_number, "max"),
                    delay: as_u32(fields[5], tag, line_number, "delay"),
                    past: as_u32(fields[7], tag, line_number, "past"),
                    threshold: f64::from_bits(as_hex64(fields[9], tag, line_number, "threshold")),
                    r: as_hex64(fields[11], tag, line_number, "r"),
                    q: as_hex64(fields[13], tag, line_number, "q"),
                    frames: Vec::new(),
                });
            }
            "frame" => {
                if fields.len() != 4 {
                    broken(tag, line_number, "frame fields");
                }
                finish_frame(&mut scenario, &mut frame);
                frame = Some(Frame {
                    number: as_usize(fields[1], tag, line_number, "frame number"),
                    detections: Vec::new(),
                    raised: false,
                    expected_tracks: 0,
                    tracks: Vec::new(),
                });
            }
            "det" => {
                if fields.len() != 7 {
                    broken(tag, line_number, "det fields");
                }
                let Some(frame) = frame.as_mut() else {
                    broken(tag, line_number, "det without frame");
                };
                frame.detections.push((
                    as_u64(fields[1], tag, line_number, "tag"),
                    fields[2].to_string(),
                    [
                        as_i32(fields[3], tag, line_number, "x1"),
                        as_i32(fields[4], tag, line_number, "y1"),
                        as_i32(fields[5], tag, line_number, "x2"),
                        as_i32(fields[6], tag, line_number, "y2"),
                    ],
                ));
            }
            "result" => {
                if fields.len() != 4 {
                    broken(tag, line_number, "result fields");
                }
                let Some(frame) = frame.as_mut() else {
                    broken(tag, line_number, "result without frame");
                };
                frame.raised = fields[1] == "nan";
                frame.expected_tracks = as_usize(fields[3], tag, line_number, "tracks");
            }
            "track" => {
                if fields.len() != 14 {
                    broken(tag, line_number, "track fields");
                }
                let Some(frame) = frame.as_mut() else {
                    broken(tag, line_number, "track without frame");
                };
                frame
                    .tracks
                    .push(parse_expected_track(&fields, tag, line_number));
            }
            "end" => {
                finish_frame(&mut scenario, &mut frame);
                let Some(owned) = scenario.take() else {
                    broken(tag, line_number, "end without scenario");
                };
                scenarios.push(owned);
            }
            other => broken(tag, line_number, &format!("unknown line kind {other}")),
        }
    }
    scenarios
}

fn replay(scenario: &Scenario) {
    let r = f64::from_bits(scenario.r);
    let q = f64::from_bits(scenario.q);
    let config = TrackConfig::new(scenario.max, scenario.delay, scenario.past, r, q)
        .unwrap_or_else(|| panic!("scenario {}: refused config", scenario.name));
    let mut tracker = Tracker::new(config, scenario.threshold);
    let mut ids = IdSource::default();
    for frame in &scenario.frames {
        let sightings: Vec<Sighting<u64>> = frame
            .detections
            .iter()
            .map(|(tag, label, edges)| Sighting {
                label: label.clone(),
                observation: Observation {
                    edges: *edges,
                    data: *tag,
                },
            })
            .collect();
        let outcome = tracker.update(&mut ids, sightings);
        assert_eq!(
            outcome.is_err(),
            frame.raised,
            "scenario {} frame {}: expected error {}",
            scenario.name,
            frame.number,
            frame.raised
        );
        let tracks = tracker.tracks();
        assert_eq!(
            tracks.len(),
            frame.expected_tracks,
            "scenario {} frame {}: track count",
            scenario.name,
            frame.number
        );
        for (position, expected) in frame.tracks.iter().enumerate() {
            compare_track(
                &tracks[position],
                expected,
                &scenario.name,
                frame.number,
                position,
            );
        }
        let expected_active: Vec<usize> = frame
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, expected)| expected.active)
            .map(|(position, _)| position)
            .collect();
        let got_active: Vec<usize> = tracker
            .active_tracks()
            .filter_map(|active| {
                tracker
                    .tracks()
                    .iter()
                    .position(|other| ptr::eq(other, active))
            })
            .collect();
        assert_eq!(
            got_active, expected_active,
            "scenario {} frame {}: active_tracks identities",
            scenario.name, frame.number
        );
    }
}

fn compare_track(
    track: &Track<u64>,
    expected: &ExpectedTrack,
    name: &str,
    frame_number: usize,
    position: usize,
) {
    assert_eq!(
        track.label(),
        expected.label,
        "scenario {name} frame {frame_number} track {position}: label"
    );
    assert_eq!(
        track.hit_counter(),
        expected.hit_counter,
        "scenario {name} frame {frame_number} track {position}: hit counter"
    );
    assert_eq!(
        track.age(),
        expected.age,
        "scenario {name} frame {frame_number} track {position}: age"
    );
    assert_eq!(
        track.is_initializing(),
        expected.initializing,
        "scenario {name} frame {frame_number} track {position}: initializing flag"
    );
    assert_eq!(
        track.initializing_id(),
        expected.initializing_id,
        "scenario {name} frame {frame_number} track {position}: initializing id"
    );
    assert_eq!(
        track.id(),
        expected.id,
        "scenario {name} frame {frame_number} track {position}: id"
    );
    assert_eq!(
        track.is_active(),
        expected.active,
        "scenario {name} frame {frame_number} track {position}: active flag"
    );
    assert_eq!(
        track.last_detection().data,
        expected.last_tag,
        "scenario {name} frame {frame_number} track {position}: last tag"
    );
    let past: Vec<(u64, u64)> = track
        .past_detections()
        .iter()
        .map(|(kept_age, observation)| (observation.data, *kept_age))
        .collect();
    assert_eq!(
        past, expected.past,
        "scenario {name} frame {frame_number} track {position}: past list"
    );
    let bits: [u64; 4] = track.estimate().map(f64::to_bits);
    assert_eq!(
        bits, expected.estimate,
        "scenario {name} frame {frame_number} track {position}: estimate bits"
    );
}

const HAND_NAMES: [&str; 14] = [
    "steady-object-becomes-real",
    "no-detections-age-out",
    "other-label-does-not-match",
    "nearest-of-two-detections-wins",
    "nearest-of-two-tracks-wins",
    "real-track-wins-over-closer-initializing",
    "real-and-initializing-matched-in-one-frame",
    "ids-follow-the-pick-order",
    "ties-go-to-the-first-detection",
    "three-new-detections-in-order",
    "pruned-track-is-replaced-with-new-ids",
    "nan-in-first-pass",
    "nan-in-second-pass",
    "empty-frames-on-an-empty-tracker",
];

#[test]
fn hand_scenarios_match_the_real_library() {
    let scenarios = parse_rows("hand rows", HAND_ROWS);
    assert_eq!(scenarios.len(), HAND_NAMES.len(), "hand scenario count");
    for (position, name) in HAND_NAMES.iter().enumerate() {
        assert_eq!(&scenarios[position].name, name, "hand scenario name");
    }
    let find = |wanted: &str| {
        scenarios
            .iter()
            .position(|scenario| scenario.name == wanted)
            .unwrap_or_else(|| panic!("hand scenario {wanted} missing"))
    };
    let pick_order = &scenarios[find("ids-follow-the-pick-order")];
    let last_frame = pick_order.frames.last().expect("pick order frames");
    assert_eq!(last_frame.tracks[0].id, Some(2), "first listed pick id");
    assert_eq!(last_frame.tracks[1].id, Some(1), "second listed pick id");
    let first_pass_nan = &scenarios[find("nan-in-first-pass")];
    let frame_two = &first_pass_nan.frames[1];
    assert!(frame_two.raised, "first pass nan frame raises");
    assert_eq!(frame_two.tracks.len(), 1, "first pass nan track count");
    assert_eq!(frame_two.tracks[0].hit_counter, 0, "first pass nan counter");
    assert_eq!(frame_two.tracks[0].age, 1, "first pass nan age");
    let second_pass_nan = &scenarios[find("nan-in-second-pass")];
    let frame_five = &second_pass_nan.frames[4];
    assert!(frame_five.raised, "second pass nan frame raises");
    assert_eq!(frame_five.tracks.len(), 2, "second pass nan track count");
    assert_eq!(
        frame_five.tracks[0].last_tag, 5,
        "second pass nan first last tag"
    );
    for scenario in &scenarios {
        replay(scenario);
    }
}

const fn probe_tracker() -> Tracker<u64> {
    let config = TrackConfig::new(25, 2, 4, 3.4, 0.1).expect("probe config");
    Tracker::new(config, 2.5)
}

fn sight(tag: u64, label: &str, edges: [i32; 4]) -> Sighting<u64> {
    Sighting {
        label: label.to_string(),
        observation: Observation { edges, data: tag },
    }
}

fn probe_frames() -> Vec<ProbePair> {
    vec![
        (
            vec![sight(0, "car", [100, 100, 140, 180])],
            vec![sight(1, "person", [400, 100, 440, 180])],
        ),
        (
            vec![sight(2, "car", [102, 100, 142, 180])],
            vec![sight(3, "person", [402, 100, 442, 180])],
        ),
        (
            vec![sight(4, "car", [104, 100, 144, 180])],
            vec![
                sight(5, "person", [404, 100, 444, 180]),
                sight(6, "person", [700, 100, 740, 180]),
            ],
        ),
        (
            vec![
                sight(7, "car", [106, 100, 146, 180]),
                sight(8, "car", [100, 300, 140, 380]),
            ],
            vec![
                sight(9, "person", [406, 100, 446, 180]),
                sight(10, "person", [702, 100, 742, 180]),
            ],
        ),
        (
            vec![
                sight(11, "car", [108, 100, 148, 180]),
                sight(12, "car", [102, 300, 142, 380]),
            ],
            vec![
                sight(13, "person", [408, 100, 448, 180]),
                sight(14, "person", [704, 100, 744, 180]),
            ],
        ),
        (
            vec![
                sight(15, "car", [110, 100, 150, 180]),
                sight(16, "car", [104, 300, 144, 380]),
            ],
            vec![
                sight(17, "person", [410, 100, 450, 180]),
                sight(18, "person", [706, 100, 746, 180]),
            ],
        ),
    ]
}

fn probe_expected() -> [(u64, Vec<TagId>, Vec<TagId>); 6] {
    [
        (1, vec![(0, None)], vec![(1, None)]),
        (2, vec![(2, None)], vec![(3, None)]),
        (3, vec![(4, Some(1))], vec![(5, Some(2)), (6, None)]),
        (
            4,
            vec![(7, Some(1)), (8, None)],
            vec![(9, Some(2)), (10, None)],
        ),
        (
            5,
            vec![(11, Some(1)), (12, None)],
            vec![(13, Some(2)), (14, Some(3))],
        ),
        (
            6,
            vec![(15, Some(1)), (16, Some(4))],
            vec![(17, Some(2)), (18, Some(3))],
        ),
    ]
}

#[test]
fn shared_id_source_keeps_ids_unique_across_trackers() {
    let mut cars = probe_tracker();
    let mut people = probe_tracker();
    let mut ids = IdSource::default();
    let frames = probe_frames();
    let expected = probe_expected();
    for (index, (car_sightings, person_sightings)) in frames.into_iter().enumerate() {
        let (number, car_expected, person_expected) = &expected[index];
        cars.update(&mut ids, car_sightings).expect("car update");
        people
            .update(&mut ids, person_sightings)
            .expect("person update");
        let car_actual: Vec<TagId> = cars
            .tracks()
            .iter()
            .map(|track| (track.last_detection().data, track.id()))
            .collect();
        let person_actual: Vec<TagId> = people
            .tracks()
            .iter()
            .map(|track| (track.last_detection().data, track.id()))
            .collect();
        assert_eq!(
            &car_actual, car_expected,
            "shared probe frame {number} cars"
        );
        assert_eq!(
            &person_actual, person_expected,
            "shared probe frame {number} people"
        );
        let initializing: Vec<u64> = cars
            .tracks()
            .iter()
            .chain(people.tracks().iter())
            .map(Track::<u64>::initializing_id)
            .collect();
        let mut sorted = initializing.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            initializing.len(),
            "shared probe frame {number} initializing ids must be distinct"
        );
    }
}

#[test]
fn update_never_panics_on_extreme_input() {
    let thresholds = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        f64::MAX,
    ];
    let labels = ["car", "person", "dog"];
    for threshold in thresholds {
        let config = TrackConfig::new(3, 2, 4, 3.4, 0.1).expect("extreme config");
        let mut tracker = Tracker::new(config, threshold);
        let mut ids = IdSource::default();
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        for _frame in 0..1000 {
            let mut sightings: Vec<Sighting<u64>> = Vec::new();
            for slot in 0..20 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let value = state >> 33;
                let edges = match value % 6 {
                    0 => [i32::MIN, i32::MIN, i32::MAX, i32::MAX],
                    1 => [0, 0, 0, 0],
                    2 => [500, 500, 400, 300],
                    3 => [0, 10, 0, 190],
                    4 => {
                        let x = ((value >> 7) % 4001) as i32 - 2000;
                        let y = ((value >> 17) % 3001) as i32 - 1500;
                        [x, y, x + ((value >> 27) % 300) as i32, y + 5]
                    }
                    _ => {
                        let x = ((value >> 7) % 4001) as i32 - 2000;
                        let y = ((value >> 17) % 3001) as i32 - 1500;
                        [x + 50, y + 50, x, y]
                    }
                };
                sightings.push(Sighting {
                    label: labels[slot % 3].to_string(),
                    observation: Observation { edges, data: value },
                });
            }
            let _ = tracker.update(&mut ids, sightings);
        }
        assert!(
            tracker.tracks().len() <= 20 * 3,
            "extreme input left {} tracks alive",
            tracker.tracks().len()
        );
    }
}

fn header_map(text: &str, marker: &str) -> BTreeMap<String, u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(marker) {
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            let mut map = BTreeMap::new();
            let mut index = 0;
            while index + 1 < tokens.len() {
                map.insert(
                    tokens[index].to_string(),
                    tokens[index + 1].parse().expect("header number"),
                );
                index += 2;
            }
            return map;
        }
    }
    panic!("fixture header has no {marker} line");
}

fn bump_stat(stats: &mut BTreeMap<String, u64>, key: &str) {
    *stats.entry(key.to_string()).or_insert(0) += 1;
}

fn recompute(scenarios: &[Scenario]) -> BTreeMap<String, u64> {
    let mut stats: BTreeMap<String, u64> = BTreeMap::new();
    for scenario in scenarios {
        bump_stat(&mut stats, "scenarios");
        if scenario.max == 1 {
            bump_stat(&mut stats, "max1");
        }
        if scenario.past == 0 {
            bump_stat(&mut stats, "past0");
        }
        if scenario.delay <= 3 {
            bump_stat(&mut stats, &format!("delay{}", scenario.delay));
        }
        if f64::from(round_f64_to_f32(scenario.threshold)).to_bits() != scenario.threshold.to_bits()
        {
            bump_stat(&mut stats, "f32_threshold");
        }
        let mut previous: Vec<PrevRow> = Vec::new();
        for frame in &scenario.frames {
            previous = frame_stats(&mut stats, frame, previous);
        }
    }
    stats
}

type PrevRow = (u64, bool, u64, String);

fn frame_stats(
    stats: &mut BTreeMap<String, u64>,
    frame: &Frame,
    mut previous: Vec<PrevRow>,
) -> Vec<PrevRow> {
    bump_stat(stats, "frames");
    if frame.detections.is_empty() {
        bump_stat(stats, "empty_frames");
    }
    if frame.detections.len() >= 3 {
        bump_stat(stats, "triple_frames");
    }
    let mut current: Vec<(u64, bool, u64, String, Option<u64>)> = Vec::new();
    for track in &frame.tracks {
        if track.hit_counter < 0 {
            bump_stat(stats, "negative_track_frames");
        }
        current.push((
            track.initializing_id,
            track.initializing,
            track.last_tag,
            track.label.clone(),
            track.id,
        ));
    }
    let previous_map: BTreeMap<u64, (bool, u64, String)> = previous
        .iter()
        .map(|row| (row.0, (row.1, row.2, row.3.clone())))
        .collect();
    let current_set: BTreeMap<u64, (bool, u64, String, Option<u64>)> = current
        .iter()
        .map(|row| (row.0, (row.1, row.2, row.3.clone(), row.4)))
        .collect();
    let created: Vec<(u64, String)> = current
        .iter()
        .filter(|row| !previous_map.contains_key(&row.0))
        .map(|row| (row.0, row.3.clone()))
        .collect();
    if !created.is_empty() {
        *stats.entry("created".to_string()).or_insert(0) += created.len() as u64;
        let previous_labels: BTreeSet<String> = previous.iter().map(|row| row.3.clone()).collect();
        if created
            .iter()
            .any(|(_, label)| previous_labels.contains(label))
        {
            bump_stat(stats, "same_label_created_frames");
        }
    }
    let removed = previous
        .iter()
        .filter(|row| !current_set.contains_key(&row.0))
        .count() as u64;
    *stats.entry("removed".to_string()).or_insert(0) += removed;
    let mut hits = 0;
    for row in &current {
        let Some(old) = previous_map.get(&row.0) else {
            continue;
        };
        if old.1 == row.2 {
            continue;
        }
        hits += 1;
        if old.0 {
            bump_stat(stats, "second_pass_hits");
        } else {
            bump_stat(stats, "first_pass_hits");
        }
    }
    if hits >= 2 {
        bump_stat(stats, "two_hit_frames");
    }
    if frame.raised {
        bump_stat(stats, "nan_frames");
        if hits >= 1 {
            bump_stat(stats, "nan_with_hits");
        } else {
            bump_stat(stats, "nan_without_hits");
        }
    }
    let left: Vec<(u64, Option<u64>)> = current
        .iter()
        .filter(|row| previous_map.get(&row.0).is_some_and(|old| old.0) && !row.1)
        .map(|row| (row.0, row.4))
        .collect();
    *stats.entry("left_initializing".to_string()).or_insert(0) += left.len() as u64;
    if left.len() >= 2 {
        bump_stat(stats, "two_left_frames");
        let ids: Vec<u64> = left.iter().filter_map(|row| row.1).collect();
        if ids.windows(2).any(|pair| pair[0] > pair[1]) {
            bump_stat(stats, "pick_order_frames");
        }
    }
    previous = current
        .into_iter()
        .map(|row| (row.0, row.1, row.2, row.3))
        .collect();
    previous
}

const STATED_FLOORS: [(&str, u64); 24] = [
    ("scenarios", 120),
    ("frames", 5000),
    ("empty_frames", 400),
    ("triple_frames", 1000),
    ("created", 1500),
    ("removed", 600),
    ("first_pass_hits", 3000),
    ("second_pass_hits", 1000),
    ("left_initializing", 500),
    ("two_left_frames", 30),
    ("pick_order_frames", 10),
    ("same_label_created_frames", 600),
    ("two_hit_frames", 1000),
    ("negative_track_frames", 250),
    ("nan_frames", 40),
    ("nan_with_hits", 15),
    ("nan_without_hits", 15),
    ("delay0", 12),
    ("delay1", 12),
    ("delay2", 12),
    ("delay3", 12),
    ("max1", 6),
    ("past0", 8),
    ("f32_threshold", 15),
];

#[test]
fn fixture_matches_the_real_library() {
    let scenarios = parse_rows("fixture", FIXTURE);
    assert_eq!(scenarios.len(), 120, "fixture scenario count");
    for scenario in &scenarios {
        replay(scenario);
    }
}

#[test]
fn fixture_floors_are_met_by_the_parsed_rows() {
    let scenarios = parse_rows("fixture", FIXTURE);
    let stated: BTreeMap<String, u64> = header_map(FIXTURE, "# floors ");
    for (name, floor) in STATED_FLOORS {
        assert_eq!(stated.get(name), Some(&floor), "fixture floors {name}");
    }
    let declared: BTreeMap<String, u64> = header_map(FIXTURE, "# counts ");
    let recomputed = recompute(&scenarios);
    for (name, value) in &declared {
        assert_eq!(
            recomputed.get(name),
            Some(value),
            "fixture counts {name} disagrees with the parsed rows"
        );
    }
    for (name, floor) in STATED_FLOORS {
        assert!(
            recomputed.get(name).copied().unwrap_or(0) >= floor,
            "fixture recomputed {name} below its floor"
        );
    }
}
