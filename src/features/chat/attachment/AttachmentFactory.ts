import { AttachmentType } from './types/AttachmentType';
import ImageAttachment from './types/ImageAttachment.vue';
import VideoAttachment from './types/VideoAttachment.vue';
import AudioAttachment from './types/AudioAttachment.vue';
import DocumentAttachment from './types/DocumentAttachment.vue';
import CodeAttachment from './types/CodeAttachment.vue';
import TextAttachment from './types/TextAttachment.vue';
import OtherAttachment from './types/OtherAttachment.vue';
import type { Component } from 'vue';

const componentMap = new Map<AttachmentType, Component>();

componentMap.set(AttachmentType.IMAGE, ImageAttachment);
componentMap.set(AttachmentType.VIDEO, VideoAttachment);
componentMap.set(AttachmentType.AUDIO, AudioAttachment);
componentMap.set(AttachmentType.DOCUMENT, DocumentAttachment);
componentMap.set(AttachmentType.CODE, CodeAttachment);
componentMap.set(AttachmentType.TEXT, TextAttachment);
componentMap.set(AttachmentType.OTHER, OtherAttachment);

export class AttachmentFactory {
  /**
   * Creates a component instance based on attachment type
   */
  static createComponent(type: AttachmentType): Component | null {
    return componentMap.get(type) || null;
  }

  /**
   * Gets all registered component types
   */
  static getRegisteredTypes(): AttachmentType[] {
    return Array.from(componentMap.keys());
  }

  /**
   * Checks if a component is registered for a specific type
   */
  static hasComponent(type: AttachmentType): boolean {
    return componentMap.has(type);
  }
}
