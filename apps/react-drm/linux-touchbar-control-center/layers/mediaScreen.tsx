import React from 'react';
import path from 'path';
import { Box, Button, KEY, Svg } from 'react-drm';
import {
  MdClose,
  MdBrightness4, MdBrightness7,
  MdMicOff,
  MdSearch,
  MdSkipPrevious, MdPlayArrow, MdSkipNext,
  MdVolumeOff, MdVolumeDown, MdVolumeUp,
  MdApps,
} from 'react-icons/md';
import { BackButton } from '../components/BackButton';
import { keys } from '../services/keyInjector';

const isBuilt = __dirname.includes(path.sep + 'dist' + path.sep);

const KBD_ILLUM_DOWN_ICON = isBuilt
  ? path.join(__dirname, '..', '..', 'assets', 'kbd_illum_down.svg')
  : path.join(__dirname, '..', 'assets', 'kbd_illum_down.svg');

const KBD_ILLUM_UP_ICON = isBuilt
  ? path.join(__dirname, '..', '..', 'assets', 'kbd_illum_up.svg')
  : path.join(__dirname, '..', 'assets', 'kbd_illum_up.svg');

// ── Actions ────────────────────────────────────────────────────────────────────

type Action =
  | 'Macro1'
  | 'BrightnessDown' | 'BrightnessUp'
  | 'MicMute'
  | 'Search'
  | 'IllumDown' | 'IllumUp'
  | 'PreviousSong' | 'PlayPause' | 'NextSong'
  | 'Mute' | 'VolumeDown' | 'VolumeUp'
  | 'AllApplications'
  | 'Unknown';

function run(action: Action) {
  switch (action) {
    case 'BrightnessDown':   return keys.pressKey(KEY.BRIGHTNESSDOWN);
    case 'BrightnessUp':     return keys.pressKey(KEY.BRIGHTNESSUP);
    case 'MicMute':          return keys.pressKey(KEY.MICMUTE);
    case 'Search':           return keys.pressKey(KEY.SEARCH);
    case 'IllumDown':        return keys.pressKey(KEY.KBDILLUMDOWN);
    case 'IllumUp':          return keys.pressKey(KEY.KBDILLUMUP);
    case 'PreviousSong':     return keys.pressKey(KEY.PREVIOUSSONG);
    case 'PlayPause':        return keys.pressKey(KEY.PLAYPAUSE);
    case 'NextSong':         return keys.pressKey(KEY.NEXTSONG);
    case 'Mute':             return keys.pressKey(KEY.MUTE);
    case 'VolumeDown':       return keys.pressKey(KEY.VOLUMEDOWN);
    case 'VolumeUp':         return keys.pressKey(KEY.VOLUMEUP);
    case 'AllApplications':  return keys.pressKey(KEY.LEFTMETA);
  }
}

// ── Component ──────────────────────────────────────────────────────────────────

const BTN_SIZE  = 60;
const ICON_SIZE = 30;

export function MediaScreen({ width, height }: { width: number; height: number }) {
  return (
    <Box style={{ flex: 1,gap: 30 }}>

      <BackButton animation="slide-right" />



      <Box style={{flexGrow:2 , gap:6}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('BrightnessDown')}
      >
        <MdBrightness4 style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('BrightnessUp')}
      >
        <MdBrightness7 style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box>
      <Box style={{flexGrow:1}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('MicMute')}
      >
        <MdMicOff style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box>
      <Box style={{flexGrow:1}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('Search')}
      >
        <MdSearch style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box>

      <Box style={{flexGrow:2 , gap:6}}   >

      <Button
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('IllumDown')}
      >
        <Svg src={KBD_ILLUM_DOWN_ICON} width={ICON_SIZE} height={ICON_SIZE} />
      </Button>

      <Button
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('IllumUp')}
      >
        <Svg src={KBD_ILLUM_UP_ICON} width={ICON_SIZE} height={ICON_SIZE} />
      </Button>
</Box>

      <Box style={{flexGrow:3 , gap:6}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('PreviousSong')}
      >
        <MdSkipPrevious style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('PlayPause')}
      >
        <MdPlayArrow style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('NextSong')}
      >
        <MdSkipNext style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box>

      <Box style={{flexGrow:3 , gap:6}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('Mute')}
      >
        <MdVolumeOff style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('VolumeDown')}
      >
        <MdVolumeDown style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('VolumeUp')}
      >
        <MdVolumeUp style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box>

      <Box style={{flexGrow: 1}} >

      <Button
       
             color="#444444"
          activeColor="#555555"
        style={{flex:1, alignItems: 'center', justifyContent: 'center', borderRadius: 10 }}
        onClick={() => run('AllApplications')}
      >
        <MdApps style={{ width: ICON_SIZE, height: ICON_SIZE }} fill="#cccccc" stroke="none" />
      </Button>
</Box> 

    </Box>
  );
}
