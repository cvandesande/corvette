use corvette_track::{Stationary, StationaryThresholds, Verdict};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/stationary_cases.txt");

const HAND_ROWS: &str = r"scenario still-person-counts-up label person after 5 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 5 changes 0 envelope 100.0 100.0 200.0 300.0 lists 6 history 6 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 6 changes 0 envelope 100.0 100.0 200.0 300.0 lists 7 history 7 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 7 changes 0 envelope 100.0 100.0 200.0 300.0 lists 8 history 8 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 8 changes 0 envelope 100.0 100.0 200.0 300.0 lists 9 history 9 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
end
scenario far-jump-resets-everything label person after 5 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 600,400,700,600 verdict alive motionless 0 changes 1 envelope 600.0 400.0 700.0 600.0 lists 1 history 0 -
step 600,400,700,600 verdict alive motionless 1 changes 1 envelope 600.0 400.0 700.0 600.0 lists 2 history 1 600,400,700,600
step 600,400,700,600 verdict alive motionless 2 changes 1 envelope 600.0 400.0 700.0 600.0 lists 3 history 2 600,400,700,600 600,400,700,600
end
scenario walking-person-resets-each-time-and-counts-one-change label person after 50 max none size 1280 720 initial 1 100,100,200,300
step 112,100,212,300 verdict alive motionless 0 changes 1 envelope 112.0 100.0 212.0 300.0 lists 1 history 0 -
step 124,100,224,300 verdict alive motionless 1 changes 1 envelope 113.8 100.0 222.2 300.0 lists 2 history 1 124,100,224,300
step 136,100,236,300 verdict alive motionless 0 changes 1 envelope 136.0 100.0 236.0 300.0 lists 1 history 0 -
step 148,100,248,300 verdict alive motionless 1 changes 1 envelope 137.8 100.0 246.2 300.0 lists 2 history 1 148,100,248,300
step 160,100,260,300 verdict alive motionless 0 changes 1 envelope 160.0 100.0 260.0 300.0 lists 1 history 0 -
step 172,100,272,300 verdict alive motionless 1 changes 1 envelope 161.8 100.0 270.2 300.0 lists 2 history 1 172,100,272,300
step 184,100,284,300 verdict alive motionless 0 changes 1 envelope 184.0 100.0 284.0 300.0 lists 1 history 0 -
step 196,100,296,300 verdict alive motionless 1 changes 1 envelope 185.8 100.0 294.2 300.0 lists 2 history 1 196,100,296,300
end
scenario stationary-object-tolerates-a-drift-an-active-one-does-not label person after 3 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 130,100,230,300 verdict alive motionless 5 changes 0 envelope 100.0 100.0 207.5 300.0 lists 6 history 6 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 130,100,230,300
step 130,100,230,300 verdict alive motionless 6 changes 0 envelope 100.0 100.0 230.0 300.0 lists 7 history 7 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 130,100,230,300 130,100,230,300
step 130,100,230,300 verdict alive motionless 7 changes 0 envelope 100.0 100.0 230.0 300.0 lists 8 history 8 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 130,100,230,300 130,100,230,300 130,100,230,300
end
scenario same-drift-on-an-active-object-resets label person after 50 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 130,100,230,300 verdict alive motionless 5 changes 0 envelope 100.0 100.0 207.5 300.0 lists 6 history 6 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 130,100,230,300
step 130,100,230,300 verdict alive motionless 6 changes 0 envelope 100.0 100.0 230.0 300.0 lists 7 history 7 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 130,100,230,300 130,100,230,300
step 130,100,230,300 verdict alive motionless 0 changes 1 envelope 130.0 100.0 230.0 300.0 lists 1 history 0 -
end
scenario car-uses-the-dynamic-active-check label car after 50 max none size 1280 720 initial 1 300,300,500,400
step 300,300,500,400 verdict alive motionless 1 changes 0 envelope 300.0 300.0 500.0 400.0 lists 2 history 2 300,300,500,400 300,300,500,400
step 300,300,500,400 verdict alive motionless 2 changes 0 envelope 300.0 300.0 500.0 400.0 lists 3 history 3 300,300,500,400 300,300,500,400 300,300,500,400
step 320,300,520,400 verdict alive motionless 3 changes 0 envelope 300.0 300.0 511.0 400.0 lists 4 history 4 300,300,500,400 300,300,500,400 300,300,500,400 320,300,520,400
step 320,300,520,400 verdict alive motionless 4 changes 0 envelope 300.0 300.0 520.0 400.0 lists 5 history 5 300,300,500,400 300,300,500,400 300,300,500,400 320,300,520,400 320,300,520,400
step 320,300,520,400 verdict alive motionless 5 changes 0 envelope 300.0 300.0 520.0 400.0 lists 6 history 6 300,300,500,400 300,300,500,400 300,300,500,400 320,300,520,400 320,300,520,400 320,300,520,400
end
scenario package-never-counts-as-known-active label package after 50 max none size 1280 720 initial 1 100,100,150,150
step 100,100,150,150 verdict alive motionless 1 changes 0 envelope 100.0 100.0 150.0 150.0 lists 2 history 2 100,100,150,150 100,100,150,150
step 100,100,150,150 verdict alive motionless 2 changes 0 envelope 100.0 100.0 150.0 150.0 lists 3 history 3 100,100,150,150 100,100,150,150 100,100,150,150
step 800,500,850,550 verdict alive motionless 3 changes 0 envelope 100.0 100.0 534.9999999999999 369.99999999999994 lists 4 history 4 100,100,150,150 100,100,150,150 100,100,150,150 800,500,850,550
step 800,500,850,550 verdict alive motionless 0 changes 1 envelope 800.0 500.0 850.0 550.0 lists 1 history 0 -
step 800,500,850,550 verdict alive motionless 1 changes 1 envelope 800.0 500.0 850.0 550.0 lists 2 history 1 800,500,850,550
step 800,500,850,550 verdict alive motionless 2 changes 1 envelope 800.0 500.0 850.0 550.0 lists 3 history 2 800,500,850,550 800,500,850,550
end
scenario license-plate-keeps-four-boxes-and-moves-on-small-change label license_plate after 50 max none size 1280 720 initial 1 500,500,560,530
step 500,500,560,530 verdict alive motionless 1 changes 0 envelope 500.0 500.0 560.0 530.0 lists 2 history 2 500,500,560,530 500,500,560,530
step 500,500,560,530 verdict alive motionless 2 changes 0 envelope 500.0 500.0 560.0 530.0 lists 3 history 3 500,500,560,530 500,500,560,530 500,500,560,530
step 500,500,560,530 verdict alive motionless 3 changes 0 envelope 500.0 500.0 560.0 530.0 lists 4 history 4 500,500,560,530 500,500,560,530 500,500,560,530 500,500,560,530
step 506,500,566,530 verdict alive motionless 0 changes 1 envelope 506.0 500.0 566.0 530.0 lists 1 history 0 -
step 506,500,566,530 verdict alive motionless 1 changes 1 envelope 506.0 500.0 566.0 530.0 lists 2 history 1 506,500,566,530
step 506,500,566,530 verdict alive motionless 2 changes 1 envelope 506.0 500.0 566.0 530.0 lists 3 history 2 506,500,566,530 506,500,566,530
step 506,500,566,530 verdict alive motionless 3 changes 1 envelope 506.0 500.0 566.0 530.0 lists 4 history 3 506,500,566,530 506,500,566,530 506,500,566,530
end
scenario expires-after-max-frames label person after 2 max 3 size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 5 changes 0 envelope 100.0 100.0 200.0 300.0 lists 6 history 6 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict expired motionless 6 changes 0 envelope 100.0 100.0 200.0 300.0 lists 7 history 7 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
end
scenario max-frames-zero-expires-one-after-the-threshold label person after 2 max 0 size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict expired motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
end
scenario jitter-keeps-counting-and-the-envelope-stops-after-ten label person after 50 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 101,101,201,301 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.7 300.7 lists 3 history 3 100,100,200,300 100,100,200,300 101,101,201,301
step 102,100,202,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 201.55 300.55 lists 4 history 4 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300
step 100,101,200,301 verdict alive motionless 4 changes 0 envelope 100.0 100.0 201.4 301.0 lists 5 history 5 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301
step 101,100,201,300 verdict alive motionless 5 changes 0 envelope 100.0 100.0 201.25 301.0 lists 6 history 6 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300
step 102,101,202,301 verdict alive motionless 6 changes 0 envelope 100.0 100.0 202.0 301.0 lists 7 history 7 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301
step 100,100,200,300 verdict alive motionless 7 changes 0 envelope 100.0 100.0 201.95 301.0 lists 8 history 8 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300
step 101,101,201,301 verdict alive motionless 8 changes 0 envelope 100.0 100.0 201.8 301.0 lists 9 history 9 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301
step 102,100,202,300 verdict alive motionless 9 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 100,100,200,300 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300
step 100,101,200,301 verdict alive motionless 10 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301
step 101,100,201,300 verdict alive motionless 11 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300
step 102,101,202,301 verdict alive motionless 12 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301
step 100,100,200,300 verdict alive motionless 13 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300
step 101,101,201,301 verdict alive motionless 14 changes 0 envelope 100.0 100.0 202.0 301.0 lists 10 history 10 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301 102,100,202,300 100,101,200,301 101,100,201,300 102,101,202,301 100,100,200,300 101,101,201,301
end
scenario four-initial-boxes-from-past-detections label person after 50 max none size 1280 720 initial 4 100,100,190,290 102,100,192,292 101,101,195,295 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,190,290 102,100,192,292 101,101,195,295 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 6 history 6 100,100,190,290 102,100,192,292 101,101,195,295 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 7 history 7 100,100,190,290 102,100,192,292 101,101,195,295 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 8 history 8 100,100,190,290 102,100,192,292 101,101,195,295 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
end
scenario growing-box-where-the-ordinary-median-differs-from-the-upper-middle label person after 50 max none size 1280 720 initial 1 100,100,140,200
step 100,100,146,206 verdict alive motionless 1 changes 0 envelope 100.0 100.0 145.1 205.1 lists 2 history 2 100,100,140,200 100,100,146,206
step 100,100,152,212 verdict alive motionless 2 changes 0 envelope 100.0 100.0 150.2 210.2 lists 3 history 3 100,100,140,200 100,100,146,206 100,100,152,212
step 100,100,158,218 verdict alive motionless 3 changes 0 envelope 100.0 100.0 155.3 215.3 lists 4 history 4 100,100,140,200 100,100,146,206 100,100,152,212 100,100,158,218
step 100,100,164,224 verdict alive motionless 4 changes 0 envelope 100.0 100.0 160.4 220.4 lists 5 history 5 100,100,140,200 100,100,146,206 100,100,152,212 100,100,158,218 100,100,164,224
step 100,100,170,230 verdict alive motionless 0 changes 1 envelope 100.0 100.0 170.0 230.0 lists 1 history 0 -
step 100,100,176,236 verdict alive motionless 1 changes 1 envelope 100.0 100.0 175.1 235.1 lists 2 history 1 100,100,176,236
end
scenario position-change-while-stationary-counts-again label person after 2 max none size 1280 720 initial 1 100,100,200,300
step 100,100,200,300 verdict alive motionless 1 changes 0 envelope 100.0 100.0 200.0 300.0 lists 2 history 2 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 2 changes 0 envelope 100.0 100.0 200.0 300.0 lists 3 history 3 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 3 changes 0 envelope 100.0 100.0 200.0 300.0 lists 4 history 4 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 100,100,200,300 verdict alive motionless 4 changes 0 envelope 100.0 100.0 200.0 300.0 lists 5 history 5 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300 100,100,200,300
step 500,300,600,500 verdict alive motionless 0 changes 1 envelope 500.0 300.0 600.0 500.0 lists 1 history 0 -
step 500,300,600,500 verdict alive motionless 1 changes 1 envelope 500.0 300.0 600.0 500.0 lists 2 history 1 500,300,600,500
step 500,300,600,500 verdict alive motionless 2 changes 1 envelope 500.0 300.0 600.0 500.0 lists 3 history 2 500,300,600,500 500,300,600,500
step 500,300,600,500 verdict alive motionless 3 changes 1 envelope 500.0 300.0 600.0 500.0 lists 4 history 3 500,300,600,500 500,300,600,500 500,300,600,500
step 500,300,600,500 verdict alive motionless 4 changes 1 envelope 500.0 300.0 600.0 500.0 lists 5 history 4 500,300,600,500 500,300,600,500 500,300,600,500 500,300,600,500
step 100,100,200,300 verdict alive motionless 0 changes 2 envelope 100.0 100.0 200.0 300.0 lists 1 history 0 -
end
scenario median-iou-equal-to-limit-stays-put label boat after 3 max none size 200 200 initial 1 50,50,78,64
step 49,50,82,59 verdict alive motionless 1 changes 0 envelope 49.15 50.0 81.4 63.25 lists 2 history 2 50,50,78,64 49,50,82,59
step 50,45,72,65 verdict alive motionless 2 changes 0 envelope 49.3 46.5 80.8 64.7 lists 3 history 3 50,50,78,64 49,50,82,59 50,45,72,65
step 43,44,72,62 verdict alive motionless 3 changes 0 envelope 45.7 44.45 80.2 64.55 lists 4 history 4 50,50,78,64 49,50,82,59 50,45,72,65 43,44,72,62
step 53,55,74,67 verdict alive motionless 0 changes 1 envelope 53.0 55.0 74.0 67.0 lists 1 history 0 -
step 67,31,99,42 verdict alive motionless 1 changes 1 envelope 55.1 34.6 95.25 63.25 lists 2 history 1 67,31,99,42
end
scenario one-pixel-column-overlap-stays-put label bicycle after 7 max none size 743 1829 initial 7 563,1621,568,1644 562,1621,568,1642 565,1622,566,1645 565,1623,567,1642 562,1624,569,1641 565,1624,569,1643 565,1622,570,1643
step 562,1622,569,1642 verdict alive motionless 0 changes 1 envelope 562.0 1622.0 569.0 1642.0 lists 1 history 0 -
step 564,1620,567,1643 verdict alive motionless 1 changes 1 envelope 562.3 1620.3 568.7 1642.85 lists 2 history 1 564,1620,567,1643
step 562,1622,568,1641 verdict alive motionless 0 changes 1 envelope 562.0 1622.0 568.0 1641.0 lists 1 history 0 -
step 561,1623,570,1642 verdict alive motionless 1 changes 1 envelope 561.15 1622.15 569.7 1641.85 lists 2 history 1 561,1623,570,1642
step 560,1621,569,1641 verdict alive motionless 2 changes 1 envelope 560.3 1621.3 569.7 1641.7 lists 3 history 2 561,1623,570,1642 560,1621,569,1641
step 561,1622,567,1639 verdict alive motionless 3 changes 1 envelope 560.45 1621.45 569.55 1641.55 lists 4 history 3 561,1623,570,1642 560,1621,569,1641 561,1622,567,1639
step 560,1621,567,1637 verdict alive motionless 4 changes 1 envelope 560.0 1621.0 569.4 1641.4 lists 5 history 4 561,1623,570,1642 560,1621,569,1641 561,1622,567,1639 560,1621,567,1637
step 562,1620,565,1636 verdict alive motionless 0 changes 1 envelope 562.0 1620.0 565.0 1636.0 lists 1 history 0 -
step 564,1619,565,1634 verdict alive motionless 1 changes 1 envelope 562.3 1619.15 565.0 1635.7 lists 2 history 1 564,1619,565,1634
step 562,1621,567,1636 verdict alive motionless 0 changes 1 envelope 562.0 1621.0 567.0 1636.0 lists 1 history 0 -
step 562,1621,569,1635 verdict alive motionless 1 changes 1 envelope 562.0 1621.0 568.7 1635.85 lists 2 history 1 562,1621,569,1635
step 564,1620,570,1637 verdict alive motionless 2 changes 1 envelope 562.0 1620.3 569.7 1636.7 lists 3 history 2 562,1621,569,1635 564,1620,570,1637
step 565,1619,570,1635 verdict alive motionless 3 changes 1 envelope 562.0 1619.45 570.0 1636.55 lists 4 history 3 562,1621,569,1635 564,1620,570,1637 565,1619,570,1635
step 564,1619,569,1634 verdict alive motionless 4 changes 1 envelope 562.0 1619.0 570.0 1636.4 lists 5 history 4 562,1621,569,1635 564,1620,570,1637 565,1619,570,1635 564,1619,569,1634
step 562,1620,568,1635 verdict alive motionless 0 changes 1 envelope 562.0 1620.0 568.0 1635.0 lists 1 history 0 -
step 562,1622,570,1634 verdict alive motionless 1 changes 1 envelope 562.0 1620.3 569.7 1634.85 lists 2 history 1 562,1622,570,1634
step 562,1624,570,1634 verdict alive motionless 2 changes 1 envelope 562.0 1620.6 570.0 1634.7 lists 3 history 2 562,1622,570,1634 562,1624,570,1634
step 562,1625,572,1636 verdict alive motionless 3 changes 1 envelope 562.0 1620.9 571.1 1635.55 lists 4 history 3 562,1622,570,1634 562,1624,570,1634 562,1625,572,1636
step 560,1626,571,1635 verdict alive motionless 0 changes 1 envelope 560.0 1626.0 571.0 1635.0 lists 1 history 0 -
step 561,1626,571,1637 verdict alive motionless 1 changes 1 envelope 560.15 1626.0 571.0 1636.7 lists 2 history 1 561,1626,571,1637
step 562,1625,572,1638 verdict alive motionless 2 changes 1 envelope 560.3 1625.3 571.7 1637.7 lists 3 history 2 561,1626,571,1637 562,1625,572,1638
step 562,1623,572,1640 verdict alive motionless 3 changes 1 envelope 560.45 1623.9 572.0 1639.1 lists 4 history 3 561,1626,571,1637 562,1625,572,1638 562,1623,572,1640
step 563,1622,571,1641 verdict alive motionless 4 changes 1 envelope 560.6 1622.6 572.0 1640.4 lists 5 history 4 561,1626,571,1637 562,1625,572,1638 562,1623,572,1640 563,1622,571,1641
step 561,1622,571,1642 verdict alive motionless 0 changes 1 envelope 561.0 1622.0 571.0 1642.0 lists 1 history 0 -
step 559,1621,572,1642 verdict alive motionless 1 changes 1 envelope 559.3 1621.15 571.85 1642.0 lists 2 history 1 559,1621,572,1642
step 561,1620,571,1641 verdict alive motionless 2 changes 1 envelope 559.6 1620.3 571.7 1642.0 lists 3 history 2 559,1621,572,1642 561,1620,571,1641
step 561,1622,573,1643 verdict alive motionless 3 changes 1 envelope 559.9 1620.45 572.55 1642.55 lists 4 history 3 559,1621,572,1642 561,1620,571,1641 561,1622,573,1643
step 563,1623,572,1644 verdict alive motionless 4 changes 1 envelope 560.2 1620.6 572.4 1643.4 lists 5 history 4 559,1621,572,1642 561,1620,571,1641 561,1622,573,1643 563,1623,572,1644
step 562,1625,572,1642 verdict alive motionless 5 changes 1 envelope 560.5 1620.75 572.25 1643.25 lists 6 history 5 559,1621,572,1642 561,1620,571,1641 561,1622,573,1643 563,1623,572,1644 562,1625,572,1642
step 564,1624,570,1640 verdict alive motionless 6 changes 1 envelope 560.8 1620.9 572.1 1643.1 lists 7 history 6 559,1621,572,1642 561,1620,571,1641 561,1622,573,1643 563,1623,572,1644 562,1625,572,1642 564,1624,570,1640
step 564,1622,570,1638 verdict alive motionless 0 changes 1 envelope 564.0 1622.0 570.0 1638.0 lists 1 history 0 -
step 563,1624,572,1636 verdict alive motionless 1 changes 1 envelope 563.15 1622.3 571.7 1637.7 lists 2 history 1 563,1624,572,1636
step 564,1624,572,1636 verdict alive motionless 2 changes 1 envelope 563.3 1622.6 572.0 1637.4 lists 3 history 2 563,1624,572,1636 564,1624,572,1636
step 564,1626,573,1635 verdict alive motionless 3 changes 1 envelope 563.45 1622.9 572.55 1637.1 lists 4 history 3 563,1624,572,1636 564,1624,572,1636 564,1626,573,1635
step 566,1625,572,1635 verdict alive motionless 0 changes 1 envelope 566.0 1625.0 572.0 1635.0 lists 1 history 0 -
step 566,1623,574,1635 verdict alive motionless 1 changes 1 envelope 566.0 1623.3 573.7 1635.0 lists 2 history 1 566,1623,574,1635
step 565,1623,576,1633 verdict alive motionless 2 changes 1 envelope 565.3 1623.0 575.4 1635.0 lists 3 history 2 566,1623,574,1635 565,1623,576,1633
step 564,1623,576,1634 verdict alive motionless 3 changes 1 envelope 564.45 1623.0 576.0 1635.0 lists 4 history 3 566,1623,574,1635 565,1623,576,1633 564,1623,576,1634
step 566,1625,577,1634 verdict alive motionless 0 changes 1 envelope 566.0 1625.0 577.0 1634.0 lists 1 history 0 -
step 568,1624,575,1634 verdict alive motionless 1 changes 1 envelope 566.3 1624.15 576.7 1634.0 lists 2 history 1 568,1624,575,1634
step 567,1623,573,1633 verdict alive motionless 0 changes 1 envelope 567.0 1623.0 573.0 1633.0 lists 1 history 0 -
step 566,1622,571,1631 verdict alive motionless 1 changes 1 envelope 566.15 1622.15 572.7 1632.7 lists 2 history 1 566,1622,571,1631
step 565,1620,572,1631 verdict alive motionless 2 changes 1 envelope 565.3 1620.6 572.7 1632.4 lists 3 history 2 566,1622,571,1631 565,1620,572,1631
step 564,1621,573,1630 verdict alive motionless 3 changes 1 envelope 564.45 1620.45 573.0 1632.1 lists 4 history 3 566,1622,571,1631 565,1620,572,1631 564,1621,573,1630
step 562,1623,572,1631 verdict alive motionless 0 changes 1 envelope 562.0 1623.0 572.0 1631.0 lists 1 history 0 -
step 563,1622,570,1630 verdict alive motionless 1 changes 1 envelope 562.15 1622.15 571.7 1630.85 lists 2 history 1 563,1622,570,1630
step 565,1623,572,1629 verdict alive motionless 0 changes 1 envelope 565.0 1623.0 572.0 1629.0 lists 1 history 0 -
step 567,1621,571,1630 verdict alive motionless 1 changes 1 envelope 565.3 1621.3 571.85 1629.85 lists 2 history 1 567,1621,571,1630
step 566,1619,569,1630 verdict alive motionless 0 changes 1 envelope 566.0 1619.0 569.0 1630.0 lists 1 history 0 -
step 564,1619,570,1630 verdict alive motionless 1 changes 1 envelope 564.3 1619.0 569.85 1630.0 lists 2 history 1 564,1619,570,1630
step 566,1619,572,1632 verdict alive motionless 0 changes 1 envelope 566.0 1619.0 572.0 1632.0 lists 1 history 0 -
step 566,1617,571,1631 verdict alive motionless 1 changes 1 envelope 566.0 1617.3 571.85 1631.85 lists 2 history 1 566,1617,571,1631
step 568,1618,572,1632 verdict alive motionless 2 changes 1 envelope 566.0 1617.3 572.0 1632.0 lists 3 history 2 566,1617,571,1631 568,1618,572,1632
step 567,1618,571,1633 verdict alive motionless 3 changes 1 envelope 566.0 1617.45 572.0 1632.55 lists 4 history 3 566,1617,571,1631 568,1618,572,1632 567,1618,571,1633
step 565,1616,573,1635 verdict alive motionless 4 changes 1 envelope 565.6 1616.6 572.4 1633.8 lists 5 history 4 566,1617,571,1631 568,1618,572,1632 567,1618,571,1633 565,1616,573,1635
step 566,1614,574,1634 verdict alive motionless 0 changes 1 envelope 566.0 1614.0 574.0 1634.0 lists 1 history 0 -
step 564,1616,574,1634 verdict alive motionless 1 changes 1 envelope 564.3 1614.3 574.0 1634.0 lists 2 history 1 564,1616,574,1634
step 566,1616,573,1632 verdict alive motionless 2 changes 1 envelope 564.6 1614.6 574.0 1634.0 lists 3 history 2 564,1616,574,1634 566,1616,573,1632
step 567,1618,572,1632 verdict alive motionless 0 changes 1 envelope 567.0 1618.0 572.0 1632.0 lists 1 history 0 -
step 569,1616,571,1634 verdict alive motionless 1 changes 1 envelope 567.3 1616.3 571.85 1633.7 lists 2 history 1 569,1616,571,1634
step 569,1617,572,1633 verdict alive motionless 2 changes 1 envelope 567.6 1616.3 572.0 1633.7 lists 3 history 2 569,1616,571,1634 569,1617,572,1633
step 568,1616,574,1632 verdict alive motionless 0 changes 1 envelope 568.0 1616.0 574.0 1632.0 lists 1 history 0 -
step 569,1616,574,1633 verdict alive motionless 1 changes 1 envelope 568.15 1616.0 574.0 1632.85 lists 2 history 1 569,1616,574,1633
step 571,1616,576,1635 verdict alive motionless 0 changes 1 envelope 571.0 1616.0 576.0 1635.0 lists 1 history 0 -
step 571,1615,574,1635 verdict alive motionless 1 changes 1 envelope 571.0 1615.15 575.7 1635.0 lists 2 history 1 571,1615,574,1635
step 572,1613,576,1635 verdict alive motionless 2 changes 1 envelope 571.0 1613.6 576.0 1635.0 lists 3 history 2 571,1615,574,1635 572,1613,576,1635
step 571,1612,577,1637 verdict alive motionless 3 changes 1 envelope 571.0 1612.45 576.55 1636.1 lists 4 history 3 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637
step 569,1612,578,1639 verdict alive motionless 4 changes 1 envelope 570.2 1612.0 577.4 1637.8 lists 5 history 4 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639
step 569,1612,577,1640 verdict alive motionless 5 changes 1 envelope 569.0 1612.0 577.25 1639.25 lists 6 history 5 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639 569,1612,577,1640
step 569,1613,576,1639 verdict alive motionless 6 changes 1 envelope 569.0 1612.0 577.1 1639.1 lists 7 history 6 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639 569,1612,577,1640 569,1613,576,1639
step 570,1614,575,1639 verdict alive motionless 7 changes 1 envelope 569.0 1612.0 577.0 1639.0 lists 8 history 7 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639 569,1612,577,1640 569,1613,576,1639 570,1614,575,1639
step 572,1612,576,1641 verdict alive motionless 8 changes 1 envelope 569.0 1612.0 577.0 1639.8 lists 9 history 8 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639 569,1612,577,1640 569,1613,576,1639 570,1614,575,1639 572,1612,576,1641
step 572,1611,575,1640 verdict alive motionless 9 changes 1 envelope 569.0 1612.0 577.0 1640.0 lists 10 history 9 571,1615,574,1635 572,1613,576,1635 571,1612,577,1637 569,1612,578,1639 569,1612,577,1640 569,1613,576,1639 570,1614,575,1639 572,1612,576,1641 572,1611,575,1640
step 573,1612,576,1642 verdict alive motionless 0 changes 2 envelope 573.0 1612.0 576.0 1642.0 lists 1 history 0 -
step 574,1613,577,1640 verdict alive motionless 1 changes 2 envelope 573.15 1612.15 576.85 1641.7 lists 2 history 1 574,1613,577,1640
step 576,1612,578,1640 verdict alive motionless 0 changes 2 envelope 576.0 1612.0 578.0 1640.0 lists 1 history 0 -
step 578,1614,579,1641 verdict alive motionless 1 changes 2 envelope 576.3 1612.3 578.85 1640.85 lists 2 history 1 578,1614,579,1641
step 577,1614,581,1642 verdict alive motionless 0 changes 2 envelope 577.0 1614.0 581.0 1642.0 lists 1 history 0 -
step 578,1615,582,1643 verdict alive motionless 1 changes 2 envelope 577.15 1614.15 581.85 1642.85 lists 2 history 1 578,1615,582,1643
step 580,1614,583,1643 verdict alive motionless 0 changes 2 envelope 580.0 1614.0 583.0 1643.0 lists 1 history 0 -
step 582,1613,583,1642 verdict alive motionless 1 changes 2 envelope 580.3 1613.15 583.0 1642.85 lists 2 history 1 582,1613,583,1642
step 582,1613,584,1641 verdict alive motionless 2 changes 2 envelope 580.6 1613.0 583.7 1642.7 lists 3 history 2 582,1613,583,1642 582,1613,584,1641
step 582,1611,583,1643 verdict alive motionless 3 changes 2 envelope 580.9 1611.9 583.55 1643.0 lists 4 history 3 582,1613,583,1642 582,1613,584,1641 582,1611,583,1643
step 580,1609,583,1642 verdict alive motionless 0 changes 2 envelope 580.0 1609.0 583.0 1642.0 lists 1 history 0 -
step 578,1607,584,1643 verdict alive motionless 1 changes 2 envelope 578.3 1607.3 583.85 1642.85 lists 2 history 1 578,1607,584,1643
step 579,1609,583,1643 verdict alive motionless 2 changes 2 envelope 578.3 1607.6 583.7 1643.0 lists 3 history 2 578,1607,584,1643 579,1609,583,1643
step 578,1608,582,1642 verdict alive motionless 3 changes 2 envelope 578.0 1607.45 583.55 1643.0 lists 4 history 3 578,1607,584,1643 579,1609,583,1643 578,1608,582,1642
step 579,1608,581,1640 verdict alive motionless 0 changes 2 envelope 579.0 1608.0 581.0 1640.0 lists 1 history 0 -
step 579,1607,583,1641 verdict alive motionless 1 changes 2 envelope 579.0 1607.15 582.7 1640.85 lists 2 history 1 579,1607,583,1641
step 579,1605,583,1642 verdict alive motionless 2 changes 2 envelope 579.0 1605.6 583.0 1641.7 lists 3 history 2 579,1607,583,1641 579,1605,583,1642
step 580,1605,583,1644 verdict alive motionless 3 changes 2 envelope 579.0 1605.0 583.0 1643.1 lists 4 history 3 579,1607,583,1641 579,1605,583,1642 580,1605,583,1644
step 581,1606,582,1645 verdict alive motionless 4 changes 2 envelope 579.0 1605.0 583.0 1644.4 lists 5 history 4 579,1607,583,1641 579,1605,583,1642 580,1605,583,1644 581,1606,582,1645
step 583,1606,584,1643 verdict alive motionless 5 changes 2 envelope 579.0 1605.0 583.25 1644.25 lists 6 history 5 579,1607,583,1641 579,1605,583,1642 580,1605,583,1644 581,1606,582,1645 583,1606,584,1643
end
scenario one-pixel-column-overlap-from-the-left-stays-put label bicycle after 7 max none size 743 1829 initial 7 175,1621,180,1644 175,1621,181,1642 177,1622,178,1645 176,1623,178,1642 174,1624,181,1641 174,1624,178,1643 173,1622,178,1643
step 174,1622,181,1642 verdict alive motionless 0 changes 1 envelope 174.0 1622.0 181.0 1642.0 lists 1 history 0 -
step 176,1620,179,1643 verdict alive motionless 1 changes 1 envelope 174.3 1620.3 180.7 1642.85 lists 2 history 1 176,1620,179,1643
step 175,1622,181,1641 verdict alive motionless 0 changes 1 envelope 175.0 1622.0 181.0 1641.0 lists 1 history 0 -
step 173,1623,182,1642 verdict alive motionless 1 changes 1 envelope 173.3 1622.15 181.85 1641.85 lists 2 history 1 173,1623,182,1642
step 174,1621,183,1641 verdict alive motionless 2 changes 1 envelope 173.3 1621.3 182.7 1641.7 lists 3 history 2 173,1623,182,1642 174,1621,183,1641
step 176,1622,182,1639 verdict alive motionless 3 changes 1 envelope 173.45 1621.45 182.55 1641.55 lists 4 history 3 173,1623,182,1642 174,1621,183,1641 176,1622,182,1639
step 176,1621,183,1637 verdict alive motionless 4 changes 1 envelope 173.6 1621.0 183.0 1641.4 lists 5 history 4 173,1623,182,1642 174,1621,183,1641 176,1622,182,1639 176,1621,183,1637
step 178,1620,181,1636 verdict alive motionless 0 changes 1 envelope 178.0 1620.0 181.0 1636.0 lists 1 history 0 -
step 178,1619,179,1634 verdict alive motionless 1 changes 1 envelope 178.0 1619.15 180.7 1635.7 lists 2 history 1 178,1619,179,1634
step 176,1621,181,1636 verdict alive motionless 0 changes 1 envelope 176.0 1621.0 181.0 1636.0 lists 1 history 0 -
step 174,1621,181,1635 verdict alive motionless 1 changes 1 envelope 174.3 1621.0 181.0 1635.85 lists 2 history 1 174,1621,181,1635
step 173,1620,179,1637 verdict alive motionless 2 changes 1 envelope 173.3 1620.3 181.0 1636.7 lists 3 history 2 174,1621,181,1635 173,1620,179,1637
step 173,1619,178,1635 verdict alive motionless 3 changes 1 envelope 173.0 1619.45 181.0 1636.55 lists 4 history 3 174,1621,181,1635 173,1620,179,1637 173,1619,178,1635
step 174,1619,179,1634 verdict alive motionless 4 changes 1 envelope 173.0 1619.0 181.0 1636.4 lists 5 history 4 174,1621,181,1635 173,1620,179,1637 173,1619,178,1635 174,1619,179,1634
step 175,1620,181,1635 verdict alive motionless 0 changes 1 envelope 175.0 1620.0 181.0 1635.0 lists 1 history 0 -
step 173,1622,181,1634 verdict alive motionless 1 changes 1 envelope 173.3 1620.3 181.0 1634.85 lists 2 history 1 173,1622,181,1634
step 173,1624,181,1634 verdict alive motionless 2 changes 1 envelope 173.0 1620.6 181.0 1634.7 lists 3 history 2 173,1622,181,1634 173,1624,181,1634
step 171,1625,181,1636 verdict alive motionless 3 changes 1 envelope 171.9 1620.9 181.0 1635.55 lists 4 history 3 173,1622,181,1634 173,1624,181,1634 171,1625,181,1636
step 172,1626,183,1635 verdict alive motionless 0 changes 1 envelope 172.0 1626.0 183.0 1635.0 lists 1 history 0 -
step 172,1626,182,1637 verdict alive motionless 1 changes 1 envelope 172.0 1626.0 182.85 1636.7 lists 2 history 1 172,1626,182,1637
step 171,1625,181,1638 verdict alive motionless 2 changes 1 envelope 171.3 1625.3 182.7 1637.7 lists 3 history 2 172,1626,182,1637 171,1625,181,1638
step 171,1623,181,1640 verdict alive motionless 3 changes 1 envelope 171.0 1623.9 182.55 1639.1 lists 4 history 3 172,1626,182,1637 171,1625,181,1638 171,1623,181,1640
step 172,1622,180,1641 verdict alive motionless 4 changes 1 envelope 171.0 1622.6 182.4 1640.4 lists 5 history 4 172,1626,182,1637 171,1625,181,1638 171,1623,181,1640 172,1622,180,1641
step 172,1622,182,1642 verdict alive motionless 0 changes 1 envelope 172.0 1622.0 182.0 1642.0 lists 1 history 0 -
step 171,1621,184,1642 verdict alive motionless 1 changes 1 envelope 171.15 1621.15 183.7 1642.0 lists 2 history 1 171,1621,184,1642
step 172,1620,182,1641 verdict alive motionless 2 changes 1 envelope 171.3 1620.3 183.4 1642.0 lists 3 history 2 171,1621,184,1642 172,1620,182,1641
step 170,1622,182,1643 verdict alive motionless 3 changes 1 envelope 170.45 1620.45 183.1 1642.55 lists 4 history 3 171,1621,184,1642 172,1620,182,1641 170,1622,182,1643
step 171,1623,180,1644 verdict alive motionless 4 changes 1 envelope 170.6 1620.6 182.8 1643.4 lists 5 history 4 171,1621,184,1642 172,1620,182,1641 170,1622,182,1643 171,1623,180,1644
step 171,1625,181,1642 verdict alive motionless 5 changes 1 envelope 170.75 1620.75 182.5 1643.25 lists 6 history 5 171,1621,184,1642 172,1620,182,1641 170,1622,182,1643 171,1623,180,1644 171,1625,181,1642
step 173,1624,179,1640 verdict alive motionless 6 changes 1 envelope 170.9 1620.9 182.2 1643.1 lists 7 history 6 171,1621,184,1642 172,1620,182,1641 170,1622,182,1643 171,1623,180,1644 171,1625,181,1642 173,1624,179,1640
step 173,1622,179,1638 verdict alive motionless 0 changes 1 envelope 173.0 1622.0 179.0 1638.0 lists 1 history 0 -
step 171,1624,180,1636 verdict alive motionless 1 changes 1 envelope 171.3 1622.3 179.85 1637.7 lists 2 history 1 171,1624,180,1636
step 171,1624,179,1636 verdict alive motionless 2 changes 1 envelope 171.0 1622.6 179.7 1637.4 lists 3 history 2 171,1624,180,1636 171,1624,179,1636
step 170,1626,179,1635 verdict alive motionless 3 changes 1 envelope 170.45 1622.9 179.55 1637.1 lists 4 history 3 171,1624,180,1636 171,1624,179,1636 170,1626,179,1635
step 171,1625,177,1635 verdict alive motionless 0 changes 1 envelope 171.0 1625.0 177.0 1635.0 lists 1 history 0 -
step 169,1623,177,1635 verdict alive motionless 1 changes 1 envelope 169.3 1623.3 177.0 1635.0 lists 2 history 1 169,1623,177,1635
step 167,1623,178,1633 verdict alive motionless 2 changes 1 envelope 167.6 1623.0 177.7 1635.0 lists 3 history 2 169,1623,177,1635 167,1623,178,1633
step 167,1623,179,1634 verdict alive motionless 3 changes 1 envelope 167.0 1623.0 178.55 1635.0 lists 4 history 3 169,1623,177,1635 167,1623,178,1633 167,1623,179,1634
step 166,1625,177,1634 verdict alive motionless 0 changes 1 envelope 166.0 1625.0 177.0 1634.0 lists 1 history 0 -
step 168,1624,175,1634 verdict alive motionless 1 changes 1 envelope 166.3 1624.15 176.7 1634.0 lists 2 history 1 168,1624,175,1634
step 170,1623,176,1633 verdict alive motionless 0 changes 1 envelope 170.0 1623.0 176.0 1633.0 lists 1 history 0 -
step 172,1622,177,1631 verdict alive motionless 1 changes 1 envelope 170.3 1622.15 176.85 1632.7 lists 2 history 1 172,1622,177,1631
step 171,1620,178,1631 verdict alive motionless 2 changes 1 envelope 170.3 1620.6 177.7 1632.4 lists 3 history 2 172,1622,177,1631 171,1620,178,1631
step 170,1621,179,1630 verdict alive motionless 3 changes 1 envelope 170.0 1620.45 178.55 1632.1 lists 4 history 3 172,1622,177,1631 171,1620,178,1631 170,1621,179,1630
step 171,1623,181,1631 verdict alive motionless 0 changes 1 envelope 171.0 1623.0 181.0 1631.0 lists 1 history 0 -
step 173,1622,180,1630 verdict alive motionless 1 changes 1 envelope 171.3 1622.15 180.85 1630.85 lists 2 history 1 173,1622,180,1630
step 171,1623,178,1629 verdict alive motionless 0 changes 1 envelope 171.0 1623.0 178.0 1629.0 lists 1 history 0 -
step 172,1621,176,1630 verdict alive motionless 1 changes 1 envelope 171.15 1621.3 177.7 1629.85 lists 2 history 1 172,1621,176,1630
step 174,1619,177,1630 verdict alive motionless 0 changes 1 envelope 174.0 1619.0 177.0 1630.0 lists 1 history 0 -
step 173,1619,179,1630 verdict alive motionless 1 changes 1 envelope 173.15 1619.0 178.7 1630.0 lists 2 history 1 173,1619,179,1630
step 171,1619,177,1632 verdict alive motionless 0 changes 1 envelope 171.0 1619.0 177.0 1632.0 lists 1 history 0 -
step 172,1617,177,1631 verdict alive motionless 1 changes 1 envelope 171.15 1617.3 177.0 1631.85 lists 2 history 1 172,1617,177,1631
step 171,1618,175,1632 verdict alive motionless 2 changes 1 envelope 171.0 1617.3 177.0 1632.0 lists 3 history 2 172,1617,177,1631 171,1618,175,1632
step 172,1618,176,1633 verdict alive motionless 3 changes 1 envelope 171.0 1617.45 177.0 1632.55 lists 4 history 3 172,1617,177,1631 171,1618,175,1632 172,1618,176,1633
step 170,1616,178,1635 verdict alive motionless 4 changes 1 envelope 170.6 1616.6 177.4 1633.8 lists 5 history 4 172,1617,177,1631 171,1618,175,1632 172,1618,176,1633 170,1616,178,1635
step 169,1614,177,1634 verdict alive motionless 0 changes 1 envelope 169.0 1614.0 177.0 1634.0 lists 1 history 0 -
step 169,1616,179,1634 verdict alive motionless 1 changes 1 envelope 169.0 1614.3 178.7 1634.0 lists 2 history 1 169,1616,179,1634
step 170,1616,177,1632 verdict alive motionless 2 changes 1 envelope 169.0 1614.6 178.4 1634.0 lists 3 history 2 169,1616,179,1634 170,1616,177,1632
step 171,1618,176,1632 verdict alive motionless 0 changes 1 envelope 171.0 1618.0 176.0 1632.0 lists 1 history 0 -
step 172,1616,174,1634 verdict alive motionless 1 changes 1 envelope 171.15 1616.3 175.7 1633.7 lists 2 history 1 172,1616,174,1634
step 171,1617,174,1633 verdict alive motionless 2 changes 1 envelope 171.0 1616.3 175.4 1633.7 lists 3 history 2 172,1616,174,1634 171,1617,174,1633
step 169,1616,175,1632 verdict alive motionless 0 changes 1 envelope 169.0 1616.0 175.0 1632.0 lists 1 history 0 -
step 169,1616,174,1633 verdict alive motionless 1 changes 1 envelope 169.0 1616.0 174.85 1632.85 lists 2 history 1 169,1616,174,1633
step 167,1616,172,1635 verdict alive motionless 0 changes 1 envelope 167.0 1616.0 172.0 1635.0 lists 1 history 0 -
step 169,1615,172,1635 verdict alive motionless 1 changes 1 envelope 167.3 1615.15 172.0 1635.0 lists 2 history 1 169,1615,172,1635
step 167,1613,171,1635 verdict alive motionless 2 changes 1 envelope 167.0 1613.6 172.0 1635.0 lists 3 history 2 169,1615,172,1635 167,1613,171,1635
step 166,1612,172,1637 verdict alive motionless 3 changes 1 envelope 166.45 1612.45 172.0 1636.1 lists 4 history 3 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637
step 165,1612,174,1639 verdict alive motionless 4 changes 1 envelope 165.6 1612.0 172.8 1637.8 lists 5 history 4 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639
step 166,1612,174,1640 verdict alive motionless 5 changes 1 envelope 165.75 1612.0 174.0 1639.25 lists 6 history 5 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639 166,1612,174,1640
step 167,1613,174,1639 verdict alive motionless 6 changes 1 envelope 165.9 1612.0 174.0 1639.1 lists 7 history 6 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639 166,1612,174,1640 167,1613,174,1639
step 168,1614,173,1639 verdict alive motionless 7 changes 1 envelope 166.0 1612.0 174.0 1639.0 lists 8 history 7 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639 166,1612,174,1640 167,1613,174,1639 168,1614,173,1639
step 167,1612,171,1641 verdict alive motionless 8 changes 1 envelope 166.0 1612.0 174.0 1639.8 lists 9 history 8 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639 166,1612,174,1640 167,1613,174,1639 168,1614,173,1639 167,1612,171,1641
step 168,1611,171,1640 verdict alive motionless 9 changes 1 envelope 166.0 1612.0 174.0 1640.0 lists 10 history 9 169,1615,172,1635 167,1613,171,1635 166,1612,172,1637 165,1612,174,1639 166,1612,174,1640 167,1613,174,1639 168,1614,173,1639 167,1612,171,1641 168,1611,171,1640
step 167,1612,170,1642 verdict alive motionless 0 changes 2 envelope 167.0 1612.0 170.0 1642.0 lists 1 history 0 -
step 166,1613,169,1640 verdict alive motionless 1 changes 2 envelope 166.15 1612.15 169.85 1641.7 lists 2 history 1 166,1613,169,1640
step 165,1612,167,1640 verdict alive motionless 0 changes 2 envelope 165.0 1612.0 167.0 1640.0 lists 1 history 0 -
step 164,1614,165,1641 verdict alive motionless 1 changes 2 envelope 164.15 1612.3 166.7 1640.85 lists 2 history 1 164,1614,165,1641
step 162,1614,166,1642 verdict alive motionless 0 changes 2 envelope 162.0 1614.0 166.0 1642.0 lists 1 history 0 -
step 161,1615,165,1643 verdict alive motionless 1 changes 2 envelope 161.15 1614.15 165.85 1642.85 lists 2 history 1 161,1615,165,1643
step 160,1614,163,1643 verdict alive motionless 0 changes 2 envelope 160.0 1614.0 163.0 1643.0 lists 1 history 0 -
step 160,1613,161,1642 verdict alive motionless 1 changes 2 envelope 160.0 1613.15 162.7 1642.85 lists 2 history 1 160,1613,161,1642
step 159,1613,161,1641 verdict alive motionless 2 changes 2 envelope 159.3 1613.0 162.4 1642.7 lists 3 history 2 160,1613,161,1642 159,1613,161,1641
step 160,1611,161,1643 verdict alive motionless 3 changes 2 envelope 159.45 1611.9 162.1 1643.0 lists 4 history 3 160,1613,161,1642 159,1613,161,1641 160,1611,161,1643
step 160,1609,163,1642 verdict alive motionless 0 changes 2 envelope 160.0 1609.0 163.0 1642.0 lists 1 history 0 -
step 159,1607,165,1643 verdict alive motionless 1 changes 2 envelope 159.15 1607.3 164.7 1642.85 lists 2 history 1 159,1607,165,1643
step 160,1609,164,1643 verdict alive motionless 2 changes 2 envelope 159.3 1607.6 164.7 1643.0 lists 3 history 2 159,1607,165,1643 160,1609,164,1643
step 161,1608,165,1642 verdict alive motionless 3 changes 2 envelope 159.45 1607.45 165.0 1643.0 lists 4 history 3 159,1607,165,1643 160,1609,164,1643 161,1608,165,1642
step 162,1608,164,1640 verdict alive motionless 0 changes 2 envelope 162.0 1608.0 164.0 1640.0 lists 1 history 0 -
step 160,1607,164,1641 verdict alive motionless 1 changes 2 envelope 160.3 1607.15 164.0 1640.85 lists 2 history 1 160,1607,164,1641
step 160,1605,164,1642 verdict alive motionless 2 changes 2 envelope 160.0 1605.6 164.0 1641.7 lists 3 history 2 160,1607,164,1641 160,1605,164,1642
step 160,1605,163,1644 verdict alive motionless 3 changes 2 envelope 160.0 1605.0 164.0 1643.1 lists 4 history 3 160,1607,164,1641 160,1605,164,1642 160,1605,163,1644
step 161,1606,162,1645 verdict alive motionless 4 changes 2 envelope 160.0 1605.0 164.0 1644.4 lists 5 history 4 160,1607,164,1641 160,1605,164,1642 160,1605,163,1644 161,1606,162,1645
step 159,1606,160,1643 verdict alive motionless 5 changes 2 envelope 159.75 1605.0 164.0 1644.25 lists 6 history 5 160,1607,164,1641 160,1605,164,1642 160,1605,163,1644 161,1606,162,1645 159,1606,160,1643
end
scenario one-pixel-column-overlap-in-a-vertical-flip-stays-put label bicycle after 7 max none size 743 1829 initial 7 563,185,568,208 562,187,568,208 565,184,566,207 565,187,567,206 562,188,569,205 565,186,569,205 565,186,570,207
step 562,187,569,207 verdict alive motionless 0 changes 1 envelope 562.0 187.0 569.0 207.0 lists 1 history 0 -
step 564,186,567,209 verdict alive motionless 1 changes 1 envelope 562.3 186.15 568.7 208.7 lists 2 history 1 564,186,567,209
step 562,188,568,207 verdict alive motionless 0 changes 1 envelope 562.0 188.0 568.0 207.0 lists 1 history 0 -
step 561,187,570,206 verdict alive motionless 1 changes 1 envelope 561.15 187.15 569.7 206.85 lists 2 history 1 561,187,570,206
step 560,188,569,208 verdict alive motionless 2 changes 1 envelope 560.3 187.3 569.7 207.7 lists 3 history 2 561,187,570,206 560,188,569,208
step 561,190,567,207 verdict alive motionless 3 changes 1 envelope 560.45 187.45 569.55 207.55 lists 4 history 3 561,187,570,206 560,188,569,208 561,190,567,207
step 560,192,567,208 verdict alive motionless 4 changes 1 envelope 560.0 187.6 569.4 208.0 lists 5 history 4 561,187,570,206 560,188,569,208 561,190,567,207 560,192,567,208
step 562,193,565,209 verdict alive motionless 0 changes 1 envelope 562.0 193.0 565.0 209.0 lists 1 history 0 -
step 564,195,565,210 verdict alive motionless 1 changes 1 envelope 562.3 193.3 565.0 209.85 lists 2 history 1 564,195,565,210
step 562,193,567,208 verdict alive motionless 0 changes 1 envelope 562.0 193.0 567.0 208.0 lists 1 history 0 -
step 562,194,569,208 verdict alive motionless 1 changes 1 envelope 562.0 193.15 568.7 208.0 lists 2 history 1 562,194,569,208
step 564,192,570,209 verdict alive motionless 2 changes 1 envelope 562.0 192.3 569.7 208.7 lists 3 history 2 562,194,569,208 564,192,570,209
step 565,194,570,210 verdict alive motionless 3 changes 1 envelope 562.0 192.45 570.0 209.55 lists 4 history 3 562,194,569,208 564,192,570,209 565,194,570,210
step 564,195,569,210 verdict alive motionless 4 changes 1 envelope 562.0 192.6 570.0 210.0 lists 5 history 4 562,194,569,208 564,192,570,209 565,194,570,210 564,195,569,210
step 562,194,568,209 verdict alive motionless 0 changes 1 envelope 562.0 194.0 568.0 209.0 lists 1 history 0 -
step 562,195,570,207 verdict alive motionless 1 changes 1 envelope 562.0 194.15 569.7 208.7 lists 2 history 1 562,195,570,207
step 562,195,570,205 verdict alive motionless 2 changes 1 envelope 562.0 194.3 570.0 208.4 lists 3 history 2 562,195,570,207 562,195,570,205
step 562,193,572,204 verdict alive motionless 3 changes 1 envelope 562.0 193.45 571.1 208.1 lists 4 history 3 562,195,570,207 562,195,570,205 562,193,572,204
step 560,194,571,203 verdict alive motionless 0 changes 1 envelope 560.0 194.0 571.0 203.0 lists 1 history 0 -
step 561,192,571,203 verdict alive motionless 1 changes 1 envelope 560.15 192.3 571.0 203.0 lists 2 history 1 561,192,571,203
step 562,191,572,204 verdict alive motionless 2 changes 1 envelope 560.3 191.3 571.7 203.7 lists 3 history 2 561,192,571,203 562,191,572,204
step 562,189,572,206 verdict alive motionless 3 changes 1 envelope 560.45 189.9 572.0 205.1 lists 4 history 3 561,192,571,203 562,191,572,204 562,189,572,206
step 563,188,571,207 verdict alive motionless 4 changes 1 envelope 560.6 188.6 572.0 206.4 lists 5 history 4 561,192,571,203 562,191,572,204 562,189,572,206 563,188,571,207
step 561,187,571,207 verdict alive motionless 0 changes 1 envelope 561.0 187.0 571.0 207.0 lists 1 history 0 -
step 559,187,572,208 verdict alive motionless 1 changes 1 envelope 559.3 187.0 571.85 207.85 lists 2 history 1 559,187,572,208
step 561,188,571,209 verdict alive motionless 2 changes 1 envelope 559.6 187.0 571.7 208.7 lists 3 history 2 559,187,572,208 561,188,571,209
step 561,186,573,207 verdict alive motionless 3 changes 1 envelope 559.9 186.45 572.55 208.55 lists 4 history 3 559,187,572,208 561,188,571,209 561,186,573,207
step 563,185,572,206 verdict alive motionless 4 changes 1 envelope 560.2 185.6 572.4 208.4 lists 5 history 4 559,187,572,208 561,188,571,209 561,186,573,207 563,185,572,206
step 562,187,572,204 verdict alive motionless 5 changes 1 envelope 560.5 185.75 572.25 208.25 lists 6 history 5 559,187,572,208 561,188,571,209 561,186,573,207 563,185,572,206 562,187,572,204
step 564,189,570,205 verdict alive motionless 6 changes 1 envelope 560.8 185.9 572.1 208.1 lists 7 history 6 559,187,572,208 561,188,571,209 561,186,573,207 563,185,572,206 562,187,572,204 564,189,570,205
step 564,191,570,207 verdict alive motionless 0 changes 1 envelope 564.0 191.0 570.0 207.0 lists 1 history 0 -
step 563,193,572,205 verdict alive motionless 1 changes 1 envelope 563.15 191.3 571.7 206.7 lists 2 history 1 563,193,572,205
step 564,193,572,205 verdict alive motionless 2 changes 1 envelope 563.3 191.6 572.0 206.4 lists 3 history 2 563,193,572,205 564,193,572,205
step 564,194,573,203 verdict alive motionless 3 changes 1 envelope 563.45 191.9 572.55 206.1 lists 4 history 3 563,193,572,205 564,193,572,205 564,194,573,203
step 566,194,572,204 verdict alive motionless 0 changes 1 envelope 566.0 194.0 572.0 204.0 lists 1 history 0 -
step 566,194,574,206 verdict alive motionless 1 changes 1 envelope 566.0 194.0 573.7 205.7 lists 2 history 1 566,194,574,206
step 565,196,576,206 verdict alive motionless 2 changes 1 envelope 565.3 194.0 575.4 206.0 lists 3 history 2 566,194,574,206 565,196,576,206
step 564,195,576,206 verdict alive motionless 3 changes 1 envelope 564.45 194.0 576.0 206.0 lists 4 history 3 566,194,574,206 565,196,576,206 564,195,576,206
step 566,195,577,204 verdict alive motionless 0 changes 1 envelope 566.0 195.0 577.0 204.0 lists 1 history 0 -
step 568,195,575,205 verdict alive motionless 1 changes 1 envelope 566.3 195.0 576.7 204.85 lists 2 history 1 568,195,575,205
step 567,196,573,206 verdict alive motionless 0 changes 1 envelope 567.0 196.0 573.0 206.0 lists 1 history 0 -
step 566,198,571,207 verdict alive motionless 1 changes 1 envelope 566.15 196.3 572.7 206.85 lists 2 history 1 566,198,571,207
step 565,198,572,209 verdict alive motionless 2 changes 1 envelope 565.3 196.6 572.7 208.4 lists 3 history 2 566,198,571,207 565,198,572,209
step 564,199,573,208 verdict alive motionless 3 changes 1 envelope 564.45 196.9 573.0 208.55 lists 4 history 3 566,198,571,207 565,198,572,209 564,199,573,208
step 562,198,572,206 verdict alive motionless 0 changes 1 envelope 562.0 198.0 572.0 206.0 lists 1 history 0 -
step 563,199,570,207 verdict alive motionless 1 changes 1 envelope 562.15 198.15 571.7 206.85 lists 2 history 1 563,199,570,207
step 565,200,572,206 verdict alive motionless 0 changes 1 envelope 565.0 200.0 572.0 206.0 lists 1 history 0 -
step 567,199,571,208 verdict alive motionless 1 changes 1 envelope 565.3 199.15 571.85 207.7 lists 2 history 1 567,199,571,208
step 566,199,569,210 verdict alive motionless 0 changes 1 envelope 566.0 199.0 569.0 210.0 lists 1 history 0 -
step 564,199,570,210 verdict alive motionless 1 changes 1 envelope 564.3 199.0 569.85 210.0 lists 2 history 1 564,199,570,210
step 566,197,572,210 verdict alive motionless 0 changes 1 envelope 566.0 197.0 572.0 210.0 lists 1 history 0 -
step 566,198,571,212 verdict alive motionless 1 changes 1 envelope 566.0 197.15 571.85 211.7 lists 2 history 1 566,198,571,212
step 568,197,572,211 verdict alive motionless 2 changes 1 envelope 566.0 197.0 572.0 211.7 lists 3 history 2 566,198,571,212 568,197,572,211
step 567,196,571,211 verdict alive motionless 3 changes 1 envelope 566.0 196.45 572.0 211.55 lists 4 history 3 566,198,571,212 568,197,572,211 567,196,571,211
step 565,194,573,213 verdict alive motionless 4 changes 1 envelope 565.6 195.2 572.4 212.4 lists 5 history 4 566,198,571,212 568,197,572,211 567,196,571,211 565,194,573,213
step 566,195,574,215 verdict alive motionless 0 changes 1 envelope 566.0 195.0 574.0 215.0 lists 1 history 0 -
step 564,195,574,213 verdict alive motionless 1 changes 1 envelope 564.3 195.0 574.0 214.7 lists 2 history 1 564,195,574,213
step 566,197,573,213 verdict alive motionless 2 changes 1 envelope 564.6 195.0 574.0 214.4 lists 3 history 2 564,195,574,213 566,197,573,213
step 567,197,572,211 verdict alive motionless 0 changes 1 envelope 567.0 197.0 572.0 211.0 lists 1 history 0 -
step 569,195,571,213 verdict alive motionless 1 changes 1 envelope 567.3 195.3 571.85 212.7 lists 2 history 1 569,195,571,213
step 569,196,572,212 verdict alive motionless 2 changes 1 envelope 567.6 195.3 572.0 212.7 lists 3 history 2 569,195,571,213 569,196,572,212
step 568,197,574,213 verdict alive motionless 0 changes 1 envelope 568.0 197.0 574.0 213.0 lists 1 history 0 -
step 569,196,574,213 verdict alive motionless 1 changes 1 envelope 568.15 196.15 574.0 213.0 lists 2 history 1 569,196,574,213
step 571,194,576,213 verdict alive motionless 0 changes 1 envelope 571.0 194.0 576.0 213.0 lists 1 history 0 -
step 571,194,574,214 verdict alive motionless 1 changes 1 envelope 571.0 194.0 575.7 213.85 lists 2 history 1 571,194,574,214
step 572,194,576,216 verdict alive motionless 2 changes 1 envelope 571.0 194.0 576.0 215.4 lists 3 history 2 571,194,574,214 572,194,576,216
step 571,192,577,217 verdict alive motionless 3 changes 1 envelope 571.0 192.9 576.55 216.55 lists 4 history 3 571,194,574,214 572,194,576,216 571,192,577,217
step 569,190,578,217 verdict alive motionless 4 changes 1 envelope 570.2 191.2 577.4 217.0 lists 5 history 4 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217
step 569,189,577,217 verdict alive motionless 5 changes 1 envelope 569.0 189.75 577.25 217.0 lists 6 history 5 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217 569,189,577,217
step 569,190,576,216 verdict alive motionless 6 changes 1 envelope 569.0 189.9 577.1 217.0 lists 7 history 6 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217 569,189,577,217 569,190,576,216
step 570,190,575,215 verdict alive motionless 7 changes 1 envelope 569.0 190.0 577.0 217.0 lists 8 history 7 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217 569,189,577,217 569,190,576,216 570,190,575,215
step 572,188,576,217 verdict alive motionless 8 changes 1 envelope 569.0 189.2 577.0 217.0 lists 9 history 8 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217 569,189,577,217 569,190,576,216 570,190,575,215 572,188,576,217
step 572,189,575,218 verdict alive motionless 9 changes 1 envelope 569.0 189.0 577.0 217.0 lists 10 history 9 571,194,574,214 572,194,576,216 571,192,577,217 569,190,578,217 569,189,577,217 569,190,576,216 570,190,575,215 572,188,576,217 572,189,575,218
step 573,187,576,217 verdict alive motionless 0 changes 2 envelope 573.0 187.0 576.0 217.0 lists 1 history 0 -
step 574,189,577,216 verdict alive motionless 1 changes 2 envelope 573.15 187.3 576.85 216.85 lists 2 history 1 574,189,577,216
step 576,189,578,217 verdict alive motionless 0 changes 2 envelope 576.0 189.0 578.0 217.0 lists 1 history 0 -
step 578,188,579,215 verdict alive motionless 1 changes 2 envelope 576.3 188.15 578.85 216.7 lists 2 history 1 578,188,579,215
step 577,187,581,215 verdict alive motionless 0 changes 2 envelope 577.0 187.0 581.0 215.0 lists 1 history 0 -
step 578,186,582,214 verdict alive motionless 1 changes 2 envelope 577.15 186.15 581.85 214.85 lists 2 history 1 578,186,582,214
step 580,186,583,215 verdict alive motionless 0 changes 2 envelope 580.0 186.0 583.0 215.0 lists 1 history 0 -
step 582,187,583,216 verdict alive motionless 1 changes 2 envelope 580.3 186.15 583.0 215.85 lists 2 history 1 582,187,583,216
step 582,188,584,216 verdict alive motionless 2 changes 2 envelope 580.6 186.3 583.7 216.0 lists 3 history 2 582,187,583,216 582,188,584,216
step 582,186,583,218 verdict alive motionless 3 changes 2 envelope 580.9 186.0 583.55 217.1 lists 4 history 3 582,187,583,216 582,188,584,216 582,186,583,218
step 580,187,583,220 verdict alive motionless 0 changes 2 envelope 580.0 187.0 583.0 220.0 lists 1 history 0 -
step 578,186,584,222 verdict alive motionless 1 changes 2 envelope 578.3 186.15 583.85 221.7 lists 2 history 1 578,186,584,222
step 579,186,583,220 verdict alive motionless 2 changes 2 envelope 578.3 186.0 583.7 221.4 lists 3 history 2 578,186,584,222 579,186,583,220
step 578,187,582,221 verdict alive motionless 3 changes 2 envelope 578.0 186.0 583.55 221.55 lists 4 history 3 578,186,584,222 579,186,583,220 578,187,582,221
step 579,189,581,221 verdict alive motionless 0 changes 2 envelope 579.0 189.0 581.0 221.0 lists 1 history 0 -
step 579,188,583,222 verdict alive motionless 1 changes 2 envelope 579.0 188.15 582.7 221.85 lists 2 history 1 579,188,583,222
step 579,187,583,224 verdict alive motionless 2 changes 2 envelope 579.0 187.3 583.0 223.4 lists 3 history 2 579,188,583,222 579,187,583,224
step 580,185,583,224 verdict alive motionless 3 changes 2 envelope 579.0 185.9 583.0 224.0 lists 4 history 3 579,188,583,222 579,187,583,224 580,185,583,224
step 581,184,582,223 verdict alive motionless 4 changes 2 envelope 579.0 184.6 583.0 224.0 lists 5 history 4 579,188,583,222 579,187,583,224 580,185,583,224 581,184,582,223
step 583,186,584,223 verdict alive motionless 5 changes 2 envelope 579.0 184.75 583.25 224.0 lists 6 history 5 579,188,583,222 579,187,583,224 580,185,583,224 581,184,582,223 583,186,584,223
end
scenario one-pixel-row-overlap-from-the-bottom-stays-put label bicycle after 7 max none size 1829 743 initial 7 1621,563,1644,568 1621,562,1642,568 1622,565,1645,566 1623,565,1642,567 1624,562,1641,569 1624,565,1643,569 1622,565,1643,570
step 1622,562,1642,569 verdict alive motionless 0 changes 1 envelope 1622.0 562.0 1642.0 569.0 lists 1 history 0 -
step 1620,564,1643,567 verdict alive motionless 1 changes 1 envelope 1620.3 562.3 1642.85 568.7 lists 2 history 1 1620,564,1643,567
step 1622,562,1641,568 verdict alive motionless 0 changes 1 envelope 1622.0 562.0 1641.0 568.0 lists 1 history 0 -
step 1623,561,1642,570 verdict alive motionless 1 changes 1 envelope 1622.15 561.15 1641.85 569.7 lists 2 history 1 1623,561,1642,570
step 1621,560,1641,569 verdict alive motionless 2 changes 1 envelope 1621.3 560.3 1641.7 569.7 lists 3 history 2 1623,561,1642,570 1621,560,1641,569
step 1622,561,1639,567 verdict alive motionless 3 changes 1 envelope 1621.45 560.45 1641.55 569.55 lists 4 history 3 1623,561,1642,570 1621,560,1641,569 1622,561,1639,567
step 1621,560,1637,567 verdict alive motionless 4 changes 1 envelope 1621.0 560.0 1641.4 569.4 lists 5 history 4 1623,561,1642,570 1621,560,1641,569 1622,561,1639,567 1621,560,1637,567
step 1620,562,1636,565 verdict alive motionless 0 changes 1 envelope 1620.0 562.0 1636.0 565.0 lists 1 history 0 -
step 1619,564,1634,565 verdict alive motionless 1 changes 1 envelope 1619.15 562.3 1635.7 565.0 lists 2 history 1 1619,564,1634,565
step 1621,562,1636,567 verdict alive motionless 0 changes 1 envelope 1621.0 562.0 1636.0 567.0 lists 1 history 0 -
step 1621,562,1635,569 verdict alive motionless 1 changes 1 envelope 1621.0 562.0 1635.85 568.7 lists 2 history 1 1621,562,1635,569
step 1620,564,1637,570 verdict alive motionless 2 changes 1 envelope 1620.3 562.0 1636.7 569.7 lists 3 history 2 1621,562,1635,569 1620,564,1637,570
step 1619,565,1635,570 verdict alive motionless 3 changes 1 envelope 1619.45 562.0 1636.55 570.0 lists 4 history 3 1621,562,1635,569 1620,564,1637,570 1619,565,1635,570
step 1619,564,1634,569 verdict alive motionless 4 changes 1 envelope 1619.0 562.0 1636.4 570.0 lists 5 history 4 1621,562,1635,569 1620,564,1637,570 1619,565,1635,570 1619,564,1634,569
step 1620,562,1635,568 verdict alive motionless 0 changes 1 envelope 1620.0 562.0 1635.0 568.0 lists 1 history 0 -
step 1622,562,1634,570 verdict alive motionless 1 changes 1 envelope 1620.3 562.0 1634.85 569.7 lists 2 history 1 1622,562,1634,570
step 1624,562,1634,570 verdict alive motionless 2 changes 1 envelope 1620.6 562.0 1634.7 570.0 lists 3 history 2 1622,562,1634,570 1624,562,1634,570
step 1625,562,1636,572 verdict alive motionless 3 changes 1 envelope 1620.9 562.0 1635.55 571.1 lists 4 history 3 1622,562,1634,570 1624,562,1634,570 1625,562,1636,572
step 1626,560,1635,571 verdict alive motionless 0 changes 1 envelope 1626.0 560.0 1635.0 571.0 lists 1 history 0 -
step 1626,561,1637,571 verdict alive motionless 1 changes 1 envelope 1626.0 560.15 1636.7 571.0 lists 2 history 1 1626,561,1637,571
step 1625,562,1638,572 verdict alive motionless 2 changes 1 envelope 1625.3 560.3 1637.7 571.7 lists 3 history 2 1626,561,1637,571 1625,562,1638,572
step 1623,562,1640,572 verdict alive motionless 3 changes 1 envelope 1623.9 560.45 1639.1 572.0 lists 4 history 3 1626,561,1637,571 1625,562,1638,572 1623,562,1640,572
step 1622,563,1641,571 verdict alive motionless 4 changes 1 envelope 1622.6 560.6 1640.4 572.0 lists 5 history 4 1626,561,1637,571 1625,562,1638,572 1623,562,1640,572 1622,563,1641,571
step 1622,561,1642,571 verdict alive motionless 0 changes 1 envelope 1622.0 561.0 1642.0 571.0 lists 1 history 0 -
step 1621,559,1642,572 verdict alive motionless 1 changes 1 envelope 1621.15 559.3 1642.0 571.85 lists 2 history 1 1621,559,1642,572
step 1620,561,1641,571 verdict alive motionless 2 changes 1 envelope 1620.3 559.6 1642.0 571.7 lists 3 history 2 1621,559,1642,572 1620,561,1641,571
step 1622,561,1643,573 verdict alive motionless 3 changes 1 envelope 1620.45 559.9 1642.55 572.55 lists 4 history 3 1621,559,1642,572 1620,561,1641,571 1622,561,1643,573
step 1623,563,1644,572 verdict alive motionless 4 changes 1 envelope 1620.6 560.2 1643.4 572.4 lists 5 history 4 1621,559,1642,572 1620,561,1641,571 1622,561,1643,573 1623,563,1644,572
step 1625,562,1642,572 verdict alive motionless 5 changes 1 envelope 1620.75 560.5 1643.25 572.25 lists 6 history 5 1621,559,1642,572 1620,561,1641,571 1622,561,1643,573 1623,563,1644,572 1625,562,1642,572
step 1624,564,1640,570 verdict alive motionless 6 changes 1 envelope 1620.9 560.8 1643.1 572.1 lists 7 history 6 1621,559,1642,572 1620,561,1641,571 1622,561,1643,573 1623,563,1644,572 1625,562,1642,572 1624,564,1640,570
step 1622,564,1638,570 verdict alive motionless 0 changes 1 envelope 1622.0 564.0 1638.0 570.0 lists 1 history 0 -
step 1624,563,1636,572 verdict alive motionless 1 changes 1 envelope 1622.3 563.15 1637.7 571.7 lists 2 history 1 1624,563,1636,572
step 1624,564,1636,572 verdict alive motionless 2 changes 1 envelope 1622.6 563.3 1637.4 572.0 lists 3 history 2 1624,563,1636,572 1624,564,1636,572
step 1626,564,1635,573 verdict alive motionless 3 changes 1 envelope 1622.9 563.45 1637.1 572.55 lists 4 history 3 1624,563,1636,572 1624,564,1636,572 1626,564,1635,573
step 1625,566,1635,572 verdict alive motionless 0 changes 1 envelope 1625.0 566.0 1635.0 572.0 lists 1 history 0 -
step 1623,566,1635,574 verdict alive motionless 1 changes 1 envelope 1623.3 566.0 1635.0 573.7 lists 2 history 1 1623,566,1635,574
step 1623,565,1633,576 verdict alive motionless 2 changes 1 envelope 1623.0 565.3 1635.0 575.4 lists 3 history 2 1623,566,1635,574 1623,565,1633,576
step 1623,564,1634,576 verdict alive motionless 3 changes 1 envelope 1623.0 564.45 1635.0 576.0 lists 4 history 3 1623,566,1635,574 1623,565,1633,576 1623,564,1634,576
step 1625,566,1634,577 verdict alive motionless 0 changes 1 envelope 1625.0 566.0 1634.0 577.0 lists 1 history 0 -
step 1624,568,1634,575 verdict alive motionless 1 changes 1 envelope 1624.15 566.3 1634.0 576.7 lists 2 history 1 1624,568,1634,575
step 1623,567,1633,573 verdict alive motionless 0 changes 1 envelope 1623.0 567.0 1633.0 573.0 lists 1 history 0 -
step 1622,566,1631,571 verdict alive motionless 1 changes 1 envelope 1622.15 566.15 1632.7 572.7 lists 2 history 1 1622,566,1631,571
step 1620,565,1631,572 verdict alive motionless 2 changes 1 envelope 1620.6 565.3 1632.4 572.7 lists 3 history 2 1622,566,1631,571 1620,565,1631,572
step 1621,564,1630,573 verdict alive motionless 3 changes 1 envelope 1620.45 564.45 1632.1 573.0 lists 4 history 3 1622,566,1631,571 1620,565,1631,572 1621,564,1630,573
step 1623,562,1631,572 verdict alive motionless 0 changes 1 envelope 1623.0 562.0 1631.0 572.0 lists 1 history 0 -
step 1622,563,1630,570 verdict alive motionless 1 changes 1 envelope 1622.15 562.15 1630.85 571.7 lists 2 history 1 1622,563,1630,570
step 1623,565,1629,572 verdict alive motionless 0 changes 1 envelope 1623.0 565.0 1629.0 572.0 lists 1 history 0 -
step 1621,567,1630,571 verdict alive motionless 1 changes 1 envelope 1621.3 565.3 1629.85 571.85 lists 2 history 1 1621,567,1630,571
step 1619,566,1630,569 verdict alive motionless 0 changes 1 envelope 1619.0 566.0 1630.0 569.0 lists 1 history 0 -
step 1619,564,1630,570 verdict alive motionless 1 changes 1 envelope 1619.0 564.3 1630.0 569.85 lists 2 history 1 1619,564,1630,570
step 1619,566,1632,572 verdict alive motionless 0 changes 1 envelope 1619.0 566.0 1632.0 572.0 lists 1 history 0 -
step 1617,566,1631,571 verdict alive motionless 1 changes 1 envelope 1617.3 566.0 1631.85 571.85 lists 2 history 1 1617,566,1631,571
step 1618,568,1632,572 verdict alive motionless 2 changes 1 envelope 1617.3 566.0 1632.0 572.0 lists 3 history 2 1617,566,1631,571 1618,568,1632,572
step 1618,567,1633,571 verdict alive motionless 3 changes 1 envelope 1617.45 566.0 1632.55 572.0 lists 4 history 3 1617,566,1631,571 1618,568,1632,572 1618,567,1633,571
step 1616,565,1635,573 verdict alive motionless 4 changes 1 envelope 1616.6 565.6 1633.8 572.4 lists 5 history 4 1617,566,1631,571 1618,568,1632,572 1618,567,1633,571 1616,565,1635,573
step 1614,566,1634,574 verdict alive motionless 0 changes 1 envelope 1614.0 566.0 1634.0 574.0 lists 1 history 0 -
step 1616,564,1634,574 verdict alive motionless 1 changes 1 envelope 1614.3 564.3 1634.0 574.0 lists 2 history 1 1616,564,1634,574
step 1616,566,1632,573 verdict alive motionless 2 changes 1 envelope 1614.6 564.6 1634.0 574.0 lists 3 history 2 1616,564,1634,574 1616,566,1632,573
step 1618,567,1632,572 verdict alive motionless 0 changes 1 envelope 1618.0 567.0 1632.0 572.0 lists 1 history 0 -
step 1616,569,1634,571 verdict alive motionless 1 changes 1 envelope 1616.3 567.3 1633.7 571.85 lists 2 history 1 1616,569,1634,571
step 1617,569,1633,572 verdict alive motionless 2 changes 1 envelope 1616.3 567.6 1633.7 572.0 lists 3 history 2 1616,569,1634,571 1617,569,1633,572
step 1616,568,1632,574 verdict alive motionless 0 changes 1 envelope 1616.0 568.0 1632.0 574.0 lists 1 history 0 -
step 1616,569,1633,574 verdict alive motionless 1 changes 1 envelope 1616.0 568.15 1632.85 574.0 lists 2 history 1 1616,569,1633,574
step 1616,571,1635,576 verdict alive motionless 0 changes 1 envelope 1616.0 571.0 1635.0 576.0 lists 1 history 0 -
step 1615,571,1635,574 verdict alive motionless 1 changes 1 envelope 1615.15 571.0 1635.0 575.7 lists 2 history 1 1615,571,1635,574
step 1613,572,1635,576 verdict alive motionless 2 changes 1 envelope 1613.6 571.0 1635.0 576.0 lists 3 history 2 1615,571,1635,574 1613,572,1635,576
step 1612,571,1637,577 verdict alive motionless 3 changes 1 envelope 1612.45 571.0 1636.1 576.55 lists 4 history 3 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577
step 1612,569,1639,578 verdict alive motionless 4 changes 1 envelope 1612.0 570.2 1637.8 577.4 lists 5 history 4 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578
step 1612,569,1640,577 verdict alive motionless 5 changes 1 envelope 1612.0 569.0 1639.25 577.25 lists 6 history 5 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578 1612,569,1640,577
step 1613,569,1639,576 verdict alive motionless 6 changes 1 envelope 1612.0 569.0 1639.1 577.1 lists 7 history 6 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578 1612,569,1640,577 1613,569,1639,576
step 1614,570,1639,575 verdict alive motionless 7 changes 1 envelope 1612.0 569.0 1639.0 577.0 lists 8 history 7 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578 1612,569,1640,577 1613,569,1639,576 1614,570,1639,575
step 1612,572,1641,576 verdict alive motionless 8 changes 1 envelope 1612.0 569.0 1639.8 577.0 lists 9 history 8 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578 1612,569,1640,577 1613,569,1639,576 1614,570,1639,575 1612,572,1641,576
step 1611,572,1640,575 verdict alive motionless 9 changes 1 envelope 1612.0 569.0 1640.0 577.0 lists 10 history 9 1615,571,1635,574 1613,572,1635,576 1612,571,1637,577 1612,569,1639,578 1612,569,1640,577 1613,569,1639,576 1614,570,1639,575 1612,572,1641,576 1611,572,1640,575
step 1612,573,1642,576 verdict alive motionless 0 changes 2 envelope 1612.0 573.0 1642.0 576.0 lists 1 history 0 -
step 1613,574,1640,577 verdict alive motionless 1 changes 2 envelope 1612.15 573.15 1641.7 576.85 lists 2 history 1 1613,574,1640,577
step 1612,576,1640,578 verdict alive motionless 0 changes 2 envelope 1612.0 576.0 1640.0 578.0 lists 1 history 0 -
step 1614,578,1641,579 verdict alive motionless 1 changes 2 envelope 1612.3 576.3 1640.85 578.85 lists 2 history 1 1614,578,1641,579
step 1614,577,1642,581 verdict alive motionless 0 changes 2 envelope 1614.0 577.0 1642.0 581.0 lists 1 history 0 -
step 1615,578,1643,582 verdict alive motionless 1 changes 2 envelope 1614.15 577.15 1642.85 581.85 lists 2 history 1 1615,578,1643,582
step 1614,580,1643,583 verdict alive motionless 0 changes 2 envelope 1614.0 580.0 1643.0 583.0 lists 1 history 0 -
step 1613,582,1642,583 verdict alive motionless 1 changes 2 envelope 1613.15 580.3 1642.85 583.0 lists 2 history 1 1613,582,1642,583
step 1613,582,1641,584 verdict alive motionless 2 changes 2 envelope 1613.0 580.6 1642.7 583.7 lists 3 history 2 1613,582,1642,583 1613,582,1641,584
step 1611,582,1643,583 verdict alive motionless 3 changes 2 envelope 1611.9 580.9 1643.0 583.55 lists 4 history 3 1613,582,1642,583 1613,582,1641,584 1611,582,1643,583
step 1609,580,1642,583 verdict alive motionless 0 changes 2 envelope 1609.0 580.0 1642.0 583.0 lists 1 history 0 -
step 1607,578,1643,584 verdict alive motionless 1 changes 2 envelope 1607.3 578.3 1642.85 583.85 lists 2 history 1 1607,578,1643,584
step 1609,579,1643,583 verdict alive motionless 2 changes 2 envelope 1607.6 578.3 1643.0 583.7 lists 3 history 2 1607,578,1643,584 1609,579,1643,583
step 1608,578,1642,582 verdict alive motionless 3 changes 2 envelope 1607.45 578.0 1643.0 583.55 lists 4 history 3 1607,578,1643,584 1609,579,1643,583 1608,578,1642,582
step 1608,579,1640,581 verdict alive motionless 0 changes 2 envelope 1608.0 579.0 1640.0 581.0 lists 1 history 0 -
step 1607,579,1641,583 verdict alive motionless 1 changes 2 envelope 1607.15 579.0 1640.85 582.7 lists 2 history 1 1607,579,1641,583
step 1605,579,1642,583 verdict alive motionless 2 changes 2 envelope 1605.6 579.0 1641.7 583.0 lists 3 history 2 1607,579,1641,583 1605,579,1642,583
step 1605,580,1644,583 verdict alive motionless 3 changes 2 envelope 1605.0 579.0 1643.1 583.0 lists 4 history 3 1607,579,1641,583 1605,579,1642,583 1605,580,1644,583
step 1606,581,1645,582 verdict alive motionless 4 changes 2 envelope 1605.0 579.0 1644.4 583.0 lists 5 history 4 1607,579,1641,583 1605,579,1642,583 1605,580,1644,583 1606,581,1645,582
step 1606,583,1643,584 verdict alive motionless 5 changes 2 envelope 1605.0 579.0 1644.25 583.25 lists 6 history 5 1607,579,1641,583 1605,579,1642,583 1605,580,1644,583 1606,581,1645,582 1606,583,1643,584
end
scenario one-pixel-row-overlap-from-the-top-stays-put label bicycle after 7 max none size 1829 743 initial 7 1621,175,1644,180 1621,175,1642,181 1622,177,1645,178 1623,176,1642,178 1624,174,1641,181 1624,174,1643,178 1622,173,1643,178
step 1622,174,1642,181 verdict alive motionless 0 changes 1 envelope 1622.0 174.0 1642.0 181.0 lists 1 history 0 -
step 1620,176,1643,179 verdict alive motionless 1 changes 1 envelope 1620.3 174.3 1642.85 180.7 lists 2 history 1 1620,176,1643,179
step 1622,175,1641,181 verdict alive motionless 0 changes 1 envelope 1622.0 175.0 1641.0 181.0 lists 1 history 0 -
step 1623,173,1642,182 verdict alive motionless 1 changes 1 envelope 1622.15 173.3 1641.85 181.85 lists 2 history 1 1623,173,1642,182
step 1621,174,1641,183 verdict alive motionless 2 changes 1 envelope 1621.3 173.3 1641.7 182.7 lists 3 history 2 1623,173,1642,182 1621,174,1641,183
step 1622,176,1639,182 verdict alive motionless 3 changes 1 envelope 1621.45 173.45 1641.55 182.55 lists 4 history 3 1623,173,1642,182 1621,174,1641,183 1622,176,1639,182
step 1621,176,1637,183 verdict alive motionless 4 changes 1 envelope 1621.0 173.6 1641.4 183.0 lists 5 history 4 1623,173,1642,182 1621,174,1641,183 1622,176,1639,182 1621,176,1637,183
step 1620,178,1636,181 verdict alive motionless 0 changes 1 envelope 1620.0 178.0 1636.0 181.0 lists 1 history 0 -
step 1619,178,1634,179 verdict alive motionless 1 changes 1 envelope 1619.15 178.0 1635.7 180.7 lists 2 history 1 1619,178,1634,179
step 1621,176,1636,181 verdict alive motionless 0 changes 1 envelope 1621.0 176.0 1636.0 181.0 lists 1 history 0 -
step 1621,174,1635,181 verdict alive motionless 1 changes 1 envelope 1621.0 174.3 1635.85 181.0 lists 2 history 1 1621,174,1635,181
step 1620,173,1637,179 verdict alive motionless 2 changes 1 envelope 1620.3 173.3 1636.7 181.0 lists 3 history 2 1621,174,1635,181 1620,173,1637,179
step 1619,173,1635,178 verdict alive motionless 3 changes 1 envelope 1619.45 173.0 1636.55 181.0 lists 4 history 3 1621,174,1635,181 1620,173,1637,179 1619,173,1635,178
step 1619,174,1634,179 verdict alive motionless 4 changes 1 envelope 1619.0 173.0 1636.4 181.0 lists 5 history 4 1621,174,1635,181 1620,173,1637,179 1619,173,1635,178 1619,174,1634,179
step 1620,175,1635,181 verdict alive motionless 0 changes 1 envelope 1620.0 175.0 1635.0 181.0 lists 1 history 0 -
step 1622,173,1634,181 verdict alive motionless 1 changes 1 envelope 1620.3 173.3 1634.85 181.0 lists 2 history 1 1622,173,1634,181
step 1624,173,1634,181 verdict alive motionless 2 changes 1 envelope 1620.6 173.0 1634.7 181.0 lists 3 history 2 1622,173,1634,181 1624,173,1634,181
step 1625,171,1636,181 verdict alive motionless 3 changes 1 envelope 1620.9 171.9 1635.55 181.0 lists 4 history 3 1622,173,1634,181 1624,173,1634,181 1625,171,1636,181
step 1626,172,1635,183 verdict alive motionless 0 changes 1 envelope 1626.0 172.0 1635.0 183.0 lists 1 history 0 -
step 1626,172,1637,182 verdict alive motionless 1 changes 1 envelope 1626.0 172.0 1636.7 182.85 lists 2 history 1 1626,172,1637,182
step 1625,171,1638,181 verdict alive motionless 2 changes 1 envelope 1625.3 171.3 1637.7 182.7 lists 3 history 2 1626,172,1637,182 1625,171,1638,181
step 1623,171,1640,181 verdict alive motionless 3 changes 1 envelope 1623.9 171.0 1639.1 182.55 lists 4 history 3 1626,172,1637,182 1625,171,1638,181 1623,171,1640,181
step 1622,172,1641,180 verdict alive motionless 4 changes 1 envelope 1622.6 171.0 1640.4 182.4 lists 5 history 4 1626,172,1637,182 1625,171,1638,181 1623,171,1640,181 1622,172,1641,180
step 1622,172,1642,182 verdict alive motionless 0 changes 1 envelope 1622.0 172.0 1642.0 182.0 lists 1 history 0 -
step 1621,171,1642,184 verdict alive motionless 1 changes 1 envelope 1621.15 171.15 1642.0 183.7 lists 2 history 1 1621,171,1642,184
step 1620,172,1641,182 verdict alive motionless 2 changes 1 envelope 1620.3 171.3 1642.0 183.4 lists 3 history 2 1621,171,1642,184 1620,172,1641,182
step 1622,170,1643,182 verdict alive motionless 3 changes 1 envelope 1620.45 170.45 1642.55 183.1 lists 4 history 3 1621,171,1642,184 1620,172,1641,182 1622,170,1643,182
step 1623,171,1644,180 verdict alive motionless 4 changes 1 envelope 1620.6 170.6 1643.4 182.8 lists 5 history 4 1621,171,1642,184 1620,172,1641,182 1622,170,1643,182 1623,171,1644,180
step 1625,171,1642,181 verdict alive motionless 5 changes 1 envelope 1620.75 170.75 1643.25 182.5 lists 6 history 5 1621,171,1642,184 1620,172,1641,182 1622,170,1643,182 1623,171,1644,180 1625,171,1642,181
step 1624,173,1640,179 verdict alive motionless 6 changes 1 envelope 1620.9 170.9 1643.1 182.2 lists 7 history 6 1621,171,1642,184 1620,172,1641,182 1622,170,1643,182 1623,171,1644,180 1625,171,1642,181 1624,173,1640,179
step 1622,173,1638,179 verdict alive motionless 0 changes 1 envelope 1622.0 173.0 1638.0 179.0 lists 1 history 0 -
step 1624,171,1636,180 verdict alive motionless 1 changes 1 envelope 1622.3 171.3 1637.7 179.85 lists 2 history 1 1624,171,1636,180
step 1624,171,1636,179 verdict alive motionless 2 changes 1 envelope 1622.6 171.0 1637.4 179.7 lists 3 history 2 1624,171,1636,180 1624,171,1636,179
step 1626,170,1635,179 verdict alive motionless 3 changes 1 envelope 1622.9 170.45 1637.1 179.55 lists 4 history 3 1624,171,1636,180 1624,171,1636,179 1626,170,1635,179
step 1625,171,1635,177 verdict alive motionless 0 changes 1 envelope 1625.0 171.0 1635.0 177.0 lists 1 history 0 -
step 1623,169,1635,177 verdict alive motionless 1 changes 1 envelope 1623.3 169.3 1635.0 177.0 lists 2 history 1 1623,169,1635,177
step 1623,167,1633,178 verdict alive motionless 2 changes 1 envelope 1623.0 167.6 1635.0 177.7 lists 3 history 2 1623,169,1635,177 1623,167,1633,178
step 1623,167,1634,179 verdict alive motionless 3 changes 1 envelope 1623.0 167.0 1635.0 178.55 lists 4 history 3 1623,169,1635,177 1623,167,1633,178 1623,167,1634,179
step 1625,166,1634,177 verdict alive motionless 0 changes 1 envelope 1625.0 166.0 1634.0 177.0 lists 1 history 0 -
step 1624,168,1634,175 verdict alive motionless 1 changes 1 envelope 1624.15 166.3 1634.0 176.7 lists 2 history 1 1624,168,1634,175
step 1623,170,1633,176 verdict alive motionless 0 changes 1 envelope 1623.0 170.0 1633.0 176.0 lists 1 history 0 -
step 1622,172,1631,177 verdict alive motionless 1 changes 1 envelope 1622.15 170.3 1632.7 176.85 lists 2 history 1 1622,172,1631,177
step 1620,171,1631,178 verdict alive motionless 2 changes 1 envelope 1620.6 170.3 1632.4 177.7 lists 3 history 2 1622,172,1631,177 1620,171,1631,178
step 1621,170,1630,179 verdict alive motionless 3 changes 1 envelope 1620.45 170.0 1632.1 178.55 lists 4 history 3 1622,172,1631,177 1620,171,1631,178 1621,170,1630,179
step 1623,171,1631,181 verdict alive motionless 0 changes 1 envelope 1623.0 171.0 1631.0 181.0 lists 1 history 0 -
step 1622,173,1630,180 verdict alive motionless 1 changes 1 envelope 1622.15 171.3 1630.85 180.85 lists 2 history 1 1622,173,1630,180
step 1623,171,1629,178 verdict alive motionless 0 changes 1 envelope 1623.0 171.0 1629.0 178.0 lists 1 history 0 -
step 1621,172,1630,176 verdict alive motionless 1 changes 1 envelope 1621.3 171.15 1629.85 177.7 lists 2 history 1 1621,172,1630,176
step 1619,174,1630,177 verdict alive motionless 0 changes 1 envelope 1619.0 174.0 1630.0 177.0 lists 1 history 0 -
step 1619,173,1630,179 verdict alive motionless 1 changes 1 envelope 1619.0 173.15 1630.0 178.7 lists 2 history 1 1619,173,1630,179
step 1619,171,1632,177 verdict alive motionless 0 changes 1 envelope 1619.0 171.0 1632.0 177.0 lists 1 history 0 -
step 1617,172,1631,177 verdict alive motionless 1 changes 1 envelope 1617.3 171.15 1631.85 177.0 lists 2 history 1 1617,172,1631,177
step 1618,171,1632,175 verdict alive motionless 2 changes 1 envelope 1617.3 171.0 1632.0 177.0 lists 3 history 2 1617,172,1631,177 1618,171,1632,175
step 1618,172,1633,176 verdict alive motionless 3 changes 1 envelope 1617.45 171.0 1632.55 177.0 lists 4 history 3 1617,172,1631,177 1618,171,1632,175 1618,172,1633,176
step 1616,170,1635,178 verdict alive motionless 4 changes 1 envelope 1616.6 170.6 1633.8 177.4 lists 5 history 4 1617,172,1631,177 1618,171,1632,175 1618,172,1633,176 1616,170,1635,178
step 1614,169,1634,177 verdict alive motionless 0 changes 1 envelope 1614.0 169.0 1634.0 177.0 lists 1 history 0 -
step 1616,169,1634,179 verdict alive motionless 1 changes 1 envelope 1614.3 169.0 1634.0 178.7 lists 2 history 1 1616,169,1634,179
step 1616,170,1632,177 verdict alive motionless 2 changes 1 envelope 1614.6 169.0 1634.0 178.4 lists 3 history 2 1616,169,1634,179 1616,170,1632,177
step 1618,171,1632,176 verdict alive motionless 0 changes 1 envelope 1618.0 171.0 1632.0 176.0 lists 1 history 0 -
step 1616,172,1634,174 verdict alive motionless 1 changes 1 envelope 1616.3 171.15 1633.7 175.7 lists 2 history 1 1616,172,1634,174
step 1617,171,1633,174 verdict alive motionless 2 changes 1 envelope 1616.3 171.0 1633.7 175.4 lists 3 history 2 1616,172,1634,174 1617,171,1633,174
step 1616,169,1632,175 verdict alive motionless 0 changes 1 envelope 1616.0 169.0 1632.0 175.0 lists 1 history 0 -
step 1616,169,1633,174 verdict alive motionless 1 changes 1 envelope 1616.0 169.0 1632.85 174.85 lists 2 history 1 1616,169,1633,174
step 1616,167,1635,172 verdict alive motionless 0 changes 1 envelope 1616.0 167.0 1635.0 172.0 lists 1 history 0 -
step 1615,169,1635,172 verdict alive motionless 1 changes 1 envelope 1615.15 167.3 1635.0 172.0 lists 2 history 1 1615,169,1635,172
step 1613,167,1635,171 verdict alive motionless 2 changes 1 envelope 1613.6 167.0 1635.0 172.0 lists 3 history 2 1615,169,1635,172 1613,167,1635,171
step 1612,166,1637,172 verdict alive motionless 3 changes 1 envelope 1612.45 166.45 1636.1 172.0 lists 4 history 3 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172
step 1612,165,1639,174 verdict alive motionless 4 changes 1 envelope 1612.0 165.6 1637.8 172.8 lists 5 history 4 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174
step 1612,166,1640,174 verdict alive motionless 5 changes 1 envelope 1612.0 165.75 1639.25 174.0 lists 6 history 5 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174 1612,166,1640,174
step 1613,167,1639,174 verdict alive motionless 6 changes 1 envelope 1612.0 165.9 1639.1 174.0 lists 7 history 6 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174 1612,166,1640,174 1613,167,1639,174
step 1614,168,1639,173 verdict alive motionless 7 changes 1 envelope 1612.0 166.0 1639.0 174.0 lists 8 history 7 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174 1612,166,1640,174 1613,167,1639,174 1614,168,1639,173
step 1612,167,1641,171 verdict alive motionless 8 changes 1 envelope 1612.0 166.0 1639.8 174.0 lists 9 history 8 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174 1612,166,1640,174 1613,167,1639,174 1614,168,1639,173 1612,167,1641,171
step 1611,168,1640,171 verdict alive motionless 9 changes 1 envelope 1612.0 166.0 1640.0 174.0 lists 10 history 9 1615,169,1635,172 1613,167,1635,171 1612,166,1637,172 1612,165,1639,174 1612,166,1640,174 1613,167,1639,174 1614,168,1639,173 1612,167,1641,171 1611,168,1640,171
step 1612,167,1642,170 verdict alive motionless 0 changes 2 envelope 1612.0 167.0 1642.0 170.0 lists 1 history 0 -
step 1613,166,1640,169 verdict alive motionless 1 changes 2 envelope 1612.15 166.15 1641.7 169.85 lists 2 history 1 1613,166,1640,169
step 1612,165,1640,167 verdict alive motionless 0 changes 2 envelope 1612.0 165.0 1640.0 167.0 lists 1 history 0 -
step 1614,164,1641,165 verdict alive motionless 1 changes 2 envelope 1612.3 164.15 1640.85 166.7 lists 2 history 1 1614,164,1641,165
step 1614,162,1642,166 verdict alive motionless 0 changes 2 envelope 1614.0 162.0 1642.0 166.0 lists 1 history 0 -
step 1615,161,1643,165 verdict alive motionless 1 changes 2 envelope 1614.15 161.15 1642.85 165.85 lists 2 history 1 1615,161,1643,165
step 1614,160,1643,163 verdict alive motionless 0 changes 2 envelope 1614.0 160.0 1643.0 163.0 lists 1 history 0 -
step 1613,160,1642,161 verdict alive motionless 1 changes 2 envelope 1613.15 160.0 1642.85 162.7 lists 2 history 1 1613,160,1642,161
step 1613,159,1641,161 verdict alive motionless 2 changes 2 envelope 1613.0 159.3 1642.7 162.4 lists 3 history 2 1613,160,1642,161 1613,159,1641,161
step 1611,160,1643,161 verdict alive motionless 3 changes 2 envelope 1611.9 159.45 1643.0 162.1 lists 4 history 3 1613,160,1642,161 1613,159,1641,161 1611,160,1643,161
step 1609,160,1642,163 verdict alive motionless 0 changes 2 envelope 1609.0 160.0 1642.0 163.0 lists 1 history 0 -
step 1607,159,1643,165 verdict alive motionless 1 changes 2 envelope 1607.3 159.15 1642.85 164.7 lists 2 history 1 1607,159,1643,165
step 1609,160,1643,164 verdict alive motionless 2 changes 2 envelope 1607.6 159.3 1643.0 164.7 lists 3 history 2 1607,159,1643,165 1609,160,1643,164
step 1608,161,1642,165 verdict alive motionless 3 changes 2 envelope 1607.45 159.45 1643.0 165.0 lists 4 history 3 1607,159,1643,165 1609,160,1643,164 1608,161,1642,165
step 1608,162,1640,164 verdict alive motionless 0 changes 2 envelope 1608.0 162.0 1640.0 164.0 lists 1 history 0 -
step 1607,160,1641,164 verdict alive motionless 1 changes 2 envelope 1607.15 160.3 1640.85 164.0 lists 2 history 1 1607,160,1641,164
step 1605,160,1642,164 verdict alive motionless 2 changes 2 envelope 1605.6 160.0 1641.7 164.0 lists 3 history 2 1607,160,1641,164 1605,160,1642,164
step 1605,160,1644,163 verdict alive motionless 3 changes 2 envelope 1605.0 160.0 1643.1 164.0 lists 4 history 3 1607,160,1641,164 1605,160,1642,164 1605,160,1644,163
step 1606,161,1645,162 verdict alive motionless 4 changes 2 envelope 1605.0 160.0 1644.4 164.0 lists 5 history 4 1607,160,1641,164 1605,160,1642,164 1605,160,1644,163 1606,161,1645,162
step 1606,159,1643,160 verdict alive motionless 5 changes 2 envelope 1605.0 159.75 1644.25 164.0 lists 6 history 5 1607,160,1641,164 1605,160,1642,164 1605,160,1644,163 1606,161,1645,162 1606,159,1643,160
end
";

#[derive(Debug)]
struct StepRow {
    edges: [i32; 4],
    expired: bool,
    motionless: u64,
    changes: u64,
    envelope_bits: [u64; 4],
    lists: usize,
    history: Vec<[i32; 4]>,
}

#[derive(Debug)]
struct Scenario {
    name: String,
    label: String,
    after: u64,
    max_frames: Option<u64>,
    width: i32,
    height: i32,
    initial: Vec<[i32; 4]>,
    steps: Vec<StepRow>,
}

fn broken(tag: &str, line_number: usize, detail: &str) -> ! {
    panic!("{tag} line {line_number}: {detail}");
}

fn as_number<T: std::str::FromStr>(text: &str, tag: &str, line_number: usize, what: &str) -> T
where
    T::Err: std::fmt::Display,
{
    text.parse()
        .unwrap_or_else(|error| broken(tag, line_number, &format!("{what}: {error}")))
}

fn as_f64(text: &str, tag: &str, line_number: usize, what: &str) -> f64 {
    as_number(text, tag, line_number, what)
}

fn parse_box(text: &str, tag: &str, line_number: usize, what: &str) -> [i32; 4] {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 4 {
        broken(tag, line_number, what);
    }
    let mut edges = [0; 4];
    for (slot, part) in edges.iter_mut().zip(parts) {
        *slot = as_number(part, tag, line_number, what);
    }
    edges
}

fn parse_history(fields: &[&str], tag: &str, line_number: usize, count: usize) -> Vec<[i32; 4]> {
    if count == 0 {
        if fields.first() != Some(&"-") {
            broken(tag, line_number, "empty history without a dash");
        }
        return Vec::new();
    }
    fields
        .iter()
        .map(|text| parse_box(text, tag, line_number, "history box"))
        .collect()
}

fn parse_rows(tag: &str, text: &str) -> Vec<Scenario> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    let mut current: Option<Scenario> = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(' ').collect();
        match fields.first().copied() {
            Some("scenario") => {
                if fields.len() < 13
                    || fields[2] != "label"
                    || fields[4] != "after"
                    || fields[6] != "max"
                    || fields[8] != "size"
                    || fields[11] != "initial"
                {
                    broken(tag, line_number, "scenario fields");
                }
                let count: usize = as_number(fields[12], tag, line_number, "initial count");
                if fields.len() != 13 + count {
                    broken(tag, line_number, "scenario initial values");
                }
                current = Some(Scenario {
                    name: fields[1].to_string(),
                    label: fields[3].to_string(),
                    after: as_number(fields[5], tag, line_number, "after"),
                    max_frames: match fields[7] {
                        "none" => None,
                        value => Some(as_number(value, tag, line_number, "max frames")),
                    },
                    width: as_number(fields[9], tag, line_number, "frame width"),
                    height: as_number(fields[10], tag, line_number, "frame height"),
                    initial: fields[13..]
                        .iter()
                        .map(|text| parse_box(text, tag, line_number, "initial box"))
                        .collect(),
                    steps: Vec::new(),
                });
            }
            Some("step") => {
                if fields.len() < 17
                    || fields[2] != "verdict"
                    || fields[4] != "motionless"
                    || fields[6] != "changes"
                    || fields[8] != "envelope"
                    || fields[13] != "lists"
                    || fields[15] != "history"
                {
                    broken(tag, line_number, "step fields");
                }
                let history_len = as_number(fields[16], tag, line_number, "history length");
                if fields.len() != 17 + usize::max(history_len, 1) {
                    broken(tag, line_number, "step history values");
                }
                let envelope = [
                    as_f64(fields[9], tag, line_number, "envelope xmin").to_bits(),
                    as_f64(fields[10], tag, line_number, "envelope ymin").to_bits(),
                    as_f64(fields[11], tag, line_number, "envelope xmax").to_bits(),
                    as_f64(fields[12], tag, line_number, "envelope ymax").to_bits(),
                ];
                let Some(scenario) = current.as_mut() else {
                    broken(tag, line_number, "step without scenario");
                };
                scenario.steps.push(StepRow {
                    edges: parse_box(fields[1], tag, line_number, "step box"),
                    expired: match fields[3] {
                        "alive" => false,
                        "expired" => true,
                        other => broken(tag, line_number, &format!("unknown verdict {other}")),
                    },
                    motionless: as_number(fields[5], tag, line_number, "motionless"),
                    changes: as_number(fields[7], tag, line_number, "changes"),
                    envelope_bits: envelope,
                    lists: as_number(fields[14], tag, line_number, "lists"),
                    history: parse_history(&fields[17..], tag, line_number, history_len),
                });
            }
            Some("end") => {
                let Some(owned) = current.take() else {
                    broken(tag, line_number, "end without scenario");
                };
                scenarios.push(owned);
            }
            _ => broken(tag, line_number, "unknown line kind"),
        }
    }
    scenarios
}

fn replay(scenario: &Scenario, tag: &str) {
    let thresholds = StationaryThresholds::for_label(&scenario.label);
    let mut state = Stationary::new(&scenario.initial, scenario.width, scenario.height);
    for (index, step) in scenario.steps.iter().enumerate() {
        let number = index + 1;
        let verdict = state.observe(step.edges, &thresholds, scenario.after, scenario.max_frames);
        assert_eq!(
            verdict == Verdict::Expired,
            step.expired,
            "{tag} scenario {}: step {number}: verdict",
            scenario.name
        );
        assert_eq!(
            state.motionless_count(),
            step.motionless,
            "{tag} scenario {}: step {number}: motionless",
            scenario.name
        );
        assert_eq!(
            state.position_changes(),
            step.changes,
            "{tag} scenario {}: step {number}: changes",
            scenario.name
        );
        let envelope = state.envelope();
        for (slot, value) in envelope.iter().enumerate() {
            assert_eq!(
                value.to_bits(),
                step.envelope_bits[slot],
                "{tag} scenario {}: step {number}: envelope slot {slot}",
                scenario.name
            );
        }
        assert_eq!(
            state.position_samples(),
            step.lists,
            "{tag} scenario {}: step {number}: position sample count",
            scenario.name
        );
        assert_eq!(
            state.history(),
            step.history.as_slice(),
            "{tag} scenario {}: step {number}: history",
            scenario.name
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

fn bump(stats: &mut BTreeMap<String, u64>, key: &str) {
    *stats.entry(key.to_string()).or_insert(0) += 1;
}

const fn is_whole(bits: u64) -> bool {
    f64::from_bits(bits).abs().fract().to_bits() == 0
}

fn class_field(label: &str) -> &'static str {
    match label {
        "person" | "dog" => "default_class_steps",
        "car" | "truck" | "bicycle" => "dynamic_class_steps",
        "package" | "waste_bin" => "object_class_steps",
        _ => "plate_class_steps",
    }
}

fn recompute(scenarios: &[Scenario]) -> BTreeMap<String, u64> {
    let mut stats: BTreeMap<String, u64> = BTreeMap::new();
    for scenario in scenarios {
        bump(&mut stats, "scenarios");
        match scenario.initial.len() {
            1 => bump(&mut stats, "initial_one_scenarios"),
            2 => bump(&mut stats, "initial_two_scenarios"),
            3 => bump(&mut stats, "initial_three_scenarios"),
            4 => bump(&mut stats, "initial_four_scenarios"),
            _ => {}
        }
        if scenario.max_frames.is_none() {
            bump(&mut stats, "max_none_scenarios");
        }
        if scenario.max_frames == Some(0) {
            bump(&mut stats, "max_zero_scenarios");
        }
        let mut previous_motionless = 0_u64;
        let mut previous_changes = 0_u64;
        for step in &scenario.steps {
            bump(&mut stats, "steps");
            bump(&mut stats, class_field(&scenario.label));
            if previous_motionless >= scenario.after {
                bump(&mut stats, "stationary_steps");
            }
            if step.motionless == 0 && !step.expired {
                bump(&mut stats, "reset_steps");
                if previous_motionless >= scenario.after {
                    bump(&mut stats, "reset_while_stationary");
                }
                if step.changes == previous_changes {
                    bump(&mut stats, "reset_without_change");
                }
            }
            if step.changes > previous_changes {
                bump(&mut stats, "changes_rose");
            }
            if step.lists == 10 {
                bump(&mut stats, "full_lists_steps");
            }
            if step.history.len() == 10 {
                bump(&mut stats, "full_history_steps");
            }
            if scenario.label == "license_plate" && step.history.len() == 4 {
                bump(&mut stats, "plate_full_history_steps");
            }
            if step
                .envelope_bits
                .iter()
                .copied()
                .any(|bits| !is_whole(bits))
            {
                bump(&mut stats, "fractional_envelope_steps");
            }
            previous_motionless = step.motionless;
            previous_changes = step.changes;
        }
        if scenario.steps.last().is_some_and(|step| step.expired) {
            bump(&mut stats, "expired_scenarios");
        }
    }
    stats
}

const RECOMPUTED_FLOORS: [(&str, u64); 22] = [
    ("scenarios", 160),
    ("steps", 5500),
    ("expired_scenarios", 25),
    ("stationary_steps", 1600),
    ("reset_steps", 900),
    ("reset_while_stationary", 80),
    ("changes_rose", 450),
    ("reset_without_change", 100),
    ("full_lists_steps", 1200),
    ("full_history_steps", 600),
    ("plate_full_history_steps", 100),
    ("fractional_envelope_steps", 1800),
    ("default_class_steps", 800),
    ("dynamic_class_steps", 800),
    ("object_class_steps", 800),
    ("plate_class_steps", 800),
    ("max_none_scenarios", 50),
    ("max_zero_scenarios", 10),
    ("initial_one_scenarios", 15),
    ("initial_two_scenarios", 15),
    ("initial_three_scenarios", 15),
    ("initial_four_scenarios", 15),
];

const GENERATOR_ONLY_FLOORS: [(&str, u64); 2] =
    [("median_path_steps", 800), ("median_differs", 50)];

#[test]
fn fixture_matches_the_real_library() {
    let scenarios = parse_rows("fixture", FIXTURE);
    let counts: BTreeMap<String, u64> = header_map(FIXTURE, "# counts ");
    assert_eq!(scenarios.len(), 160, "fixture scenario count");
    assert_eq!(
        u64::try_from(scenarios.len()).expect("scenario count"),
        counts.get("scenarios").copied().expect("header scenarios"),
        "header scenario count disagrees"
    );
    let total: u64 = scenarios
        .iter()
        .map(|scenario| u64::try_from(scenario.steps.len()).expect("step count"))
        .sum();
    assert_eq!(
        total,
        counts.get("steps").copied().expect("header steps"),
        "header step count disagrees"
    );
    for scenario in &scenarios {
        replay(scenario, "fixture");
    }
}

#[test]
fn fixture_floors_are_met_by_the_parsed_rows() {
    let scenarios = parse_rows("fixture", FIXTURE);
    let stated: BTreeMap<String, u64> = header_map(FIXTURE, "# floors ");
    for (name, floor) in RECOMPUTED_FLOORS {
        assert_eq!(stated.get(name), Some(&floor), "fixture floors {name}");
    }
    for (name, floor) in GENERATOR_ONLY_FLOORS {
        assert_eq!(stated.get(name), Some(&floor), "fixture floors {name}");
    }
    let declared: BTreeMap<String, u64> = header_map(FIXTURE, "# counts ");
    let recomputed = recompute(&scenarios);
    for (name, floor) in RECOMPUTED_FLOORS {
        assert_eq!(
            declared.get(name),
            recomputed.get(name),
            "fixture counts {name} disagrees with the parsed rows"
        );
        assert!(
            recomputed.get(name).copied().unwrap_or(0) >= floor,
            "fixture recomputed {name} below its floor"
        );
    }
    for (name, floor) in GENERATOR_ONLY_FLOORS {
        assert!(
            declared.get(name).copied().unwrap_or(0) >= floor,
            "fixture header {name} below its floor"
        );
    }
}

#[test]
fn hand_scenarios_match_the_real_library() {
    let scenarios = parse_rows("hand rows", HAND_ROWS);
    assert_eq!(scenarios.len(), 20, "hand scenario count");
    let total: usize = scenarios.iter().map(|scenario| scenario.steps.len()).sum();
    assert_eq!(total, 567, "hand step count");
    for scenario in &scenarios {
        replay(scenario, "hand rows");
    }
}

#[test]
fn thresholds_per_label_match_the_table() {
    let default = StationaryThresholds::for_label("person");
    for label in ["person", "dog", "unseen_label"] {
        let thresholds = StationaryThresholds::for_label(label);
        assert_eq!(thresholds, default, "thresholds for {label}");
    }
    for label in ["bbq_grill", "package", "waste_bin"] {
        let thresholds = StationaryThresholds::for_label(label);
        assert_eq!(
            thresholds.known_active_iou().to_bits(),
            0.0f64.to_bits(),
            "{label}"
        );
        assert_eq!(
            thresholds.stationary_check_iou().to_bits(),
            0.6f64.to_bits(),
            "{label}"
        );
        assert_eq!(
            thresholds.active_check_iou().to_bits(),
            0.9f64.to_bits(),
            "{label}"
        );
        assert_eq!(thresholds.max_history(), 10, "{label}");
    }
    for label in ["bicycle", "boat", "car", "motorcycle", "tractor", "truck"] {
        let thresholds = StationaryThresholds::for_label(label);
        assert_eq!(
            thresholds.known_active_iou().to_bits(),
            0.2f64.to_bits(),
            "{label}"
        );
        assert_eq!(
            thresholds.stationary_check_iou().to_bits(),
            0.6f64.to_bits(),
            "{label}"
        );
        assert_eq!(
            thresholds.active_check_iou().to_bits(),
            0.75f64.to_bits(),
            "{label}"
        );
        assert_eq!(thresholds.max_history(), 10, "{label}");
    }
    let plate = StationaryThresholds::for_label("license_plate");
    assert_eq!(
        plate.known_active_iou().to_bits(),
        0.9f64.to_bits(),
        "plate"
    );
    assert_eq!(
        plate.stationary_check_iou().to_bits(),
        0.9f64.to_bits(),
        "plate"
    );
    assert_eq!(
        plate.active_check_iou().to_bits(),
        0.9f64.to_bits(),
        "plate"
    );
    assert_eq!(plate.max_history(), 4, "plate");
}

#[test]
fn new_state_before_any_observe() {
    let initial = [[10, 20, 30, 40], [12, 21, 34, 44], [11, 22, 33, 44]];
    let state = Stationary::new(&initial, 1280, 720);
    assert_eq!(state.motionless_count(), 0, "start motionless");
    assert_eq!(state.position_changes(), 0, "start changes");
    let envelope = state.envelope();
    assert_eq!(
        envelope[0].to_bits(),
        0.0f64.to_bits(),
        "start envelope xmin"
    );
    assert_eq!(
        envelope[1].to_bits(),
        0.0f64.to_bits(),
        "start envelope ymin"
    );
    assert_eq!(
        envelope[2].to_bits(),
        1280.0f64.to_bits(),
        "start envelope xmax"
    );
    assert_eq!(
        envelope[3].to_bits(),
        720.0f64.to_bits(),
        "start envelope ymax"
    );
    assert_eq!(state.history(), initial.as_slice(), "start history");
    assert_eq!(state.position_samples(), 3, "start sample count");
    let twelve = [[5, 5, 15, 15]; 12];
    let started = Stationary::new(&twelve, 640, 480);
    assert_eq!(
        started.history().to_vec(),
        twelve.to_vec(),
        "twelve boxes kept untrimmed"
    );
    assert_eq!(
        started.position_samples(),
        12,
        "twelve samples kept untrimmed"
    );
    let mut empty = Stationary::new(&[], 1280, 720);
    assert!(empty.history().is_empty(), "empty start history");
    assert_eq!(empty.position_samples(), 0, "empty start samples");
    let verdict = empty.observe(
        [7, 8, 9, 10],
        &StationaryThresholds::for_label("person"),
        5,
        None,
    );
    assert_eq!(
        verdict,
        Verdict::Alive,
        "first observation on an empty start"
    );
    assert_eq!(
        empty.history(),
        [[7, 8, 9, 10]].as_slice(),
        "first observation leaves one box"
    );
}

#[test]
fn observe_never_panics_on_extreme_input() {
    let extremes: [i32; 4] = [i32::MIN, i32::MAX, 0, -1];
    let labels = ["person", "car", "package", "license_plate", "unseen_label"];
    let afters: [u64; 2] = [0, u64::MAX];
    let caps: [Option<u64>; 3] = [None, Some(0), Some(u64::MAX)];
    let mut state = Stationary::new(&[], 64, 64);
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    for index in 0..2000_usize {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let drawn = seed >> 33;
        let edges = [
            extremes[(drawn % 4) as usize],
            extremes[((drawn >> 2) % 4) as usize],
            extremes[((drawn >> 4) % 4) as usize],
            extremes[((drawn >> 6) % 4) as usize],
        ];
        let thresholds = StationaryThresholds::for_label(labels[((drawn >> 8) % 5) as usize]);
        let after = afters[((drawn >> 11) % 2) as usize];
        let max_frames = caps[((drawn >> 13) % 3) as usize];
        let verdict = state.observe(edges, &thresholds, after, max_frames);
        assert!(
            matches!(verdict, Verdict::Alive | Verdict::Expired),
            "extreme observation {index} returned no verdict"
        );
        assert!(
            state.history().len() <= thresholds.max_history(),
            "extreme observation {index} grew the history past its cap"
        );
    }
}
